import * as vscode from "vscode";
import * as net from "net";
import { spawn, ChildProcess } from "child_process";
import { ensureJavarCli } from "./ensureCli";

let statusBar: vscode.StatusBarItem;
let coreProc: ChildProcess | undefined;
let pollTimer: NodeJS.Timeout | undefined;
let lastTelemetry = { heap: 0, managed: 0, regions: 0, project: "" };
let resolvedCli: string | undefined;

export function activate(context: vscode.ExtensionContext): void {
  statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
  statusBar.command = "javar.forceResync";
  statusBar.text = "$(flame) JavaR: Idle";
  statusBar.tooltip = "JavaR Cockpit — Force Re-sync (sidecar + telemetry only)";
  statusBar.show();
  context.subscriptions.push(statusBar);

  const regions = new RegionsProvider();
  vscode.window.registerTreeDataProvider("javar.regions", regions);

  context.subscriptions.push(
    vscode.commands.registerCommand("javar.connect", () => connectAndPoll(regions)),
    vscode.commands.registerCommand("javar.forceResync", () => forceResync()),
    vscode.commands.registerCommand("javar.startCli", () => startSidecar()),
    vscode.commands.registerCommand("javar.openDashboard", () => openDashboard()),
    vscode.commands.registerCommand("javar.installCli", () => installCli())
  );

  void bootstrap(regions);
}

async function bootstrap(regions: RegionsProvider): Promise<void> {
  const cfg = vscode.workspace.getConfiguration("javar");
  const configured = cfg.get<string>("cliPath", "javar");
  const offer = cfg.get<boolean>("autoInstallCli", true);
  resolvedCli = await ensureJavarCli(configured, offer);

  // Cockpit never launches the app JVM — only sidecar + telemetry.
  if (cfg.get<boolean>("autoStart", true) && vscode.workspace.workspaceFolders?.length) {
    void connectAndPoll(regions);
    if (resolvedCli) {
      void startSidecar(true);
    }
  }
}

async function installCli(): Promise<void> {
  const cfg = vscode.workspace.getConfiguration("javar");
  resolvedCli = undefined;
  resolvedCli = await ensureJavarCli(cfg.get<string>("cliPath", "javar"), true);
}

export function deactivate(): void {
  if (pollTimer) {
    clearInterval(pollTimer);
  }
  coreProc?.kill();
}

async function connectAndPoll(regions: RegionsProvider): Promise<void> {
  statusBar.text = "$(sync~spin) JavaR: Connecting…";
  if (pollTimer) {
    clearInterval(pollTimer);
  }
  pollTimer = setInterval(() => {
    void refreshTelemetry(regions);
  }, 2000);
  await refreshTelemetry(regions);
}

async function refreshTelemetry(regions: RegionsProvider): Promise<void> {
  const cfg = vscode.workspace.getConfiguration("javar");
  const host = cfg.get<string>("agentHost", "127.0.0.1");
  const port = cfg.get<number>("agentPort", 19222);
  try {
    const snap = await requestTelemetry(host, port);
    lastTelemetry = {
      heap: Number(snap.java_heap_used ?? 0),
      managed: Number(snap.javar_managed ?? 0),
      regions: Number(snap.managed_regions ?? 0),
      project: typeof snap.project_name === "string" ? snap.project_name : "",
    };
    const heapMb = (lastTelemetry.heap / (1024 * 1024)).toFixed(1);
    const manMb = (lastTelemetry.managed / (1024 * 1024)).toFixed(1);
    const proj = lastTelemetry.project ? ` · ${lastTelemetry.project}` : "";
    statusBar.text = `$(flame) JavaR: Active${proj} · Heap ${heapMb}MB · Off-heap ${manMb}MB`;
    regions.update(lastTelemetry.regions, lastTelemetry.managed, lastTelemetry.project);
  } catch {
    statusBar.text = "$(flame) JavaR: Watching (agent offline)";
  }
}

async function forceResync(): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (editor?.document.isDirty) {
    await editor.document.save();
  }
  const cfg = vscode.workspace.getConfiguration("javar");
  const host = cfg.get<string>("agentHost", "127.0.0.1");
  const port = cfg.get<number>("agentPort", 19222);
  const path = editor?.document.uri.fsPath ?? "";
  try {
    await sendHotDeploy(host, port, path);
    vscode.window.setStatusBarMessage("JavaR: Force Re-sync sent", 2500);
    statusBar.text = "$(sync) JavaR: Re-sync…";
  } catch (e) {
    vscode.window.showErrorMessage(`JavaR Force Re-sync failed: ${String(e)}`);
  }
}

async function resolveCliPath(): Promise<string | undefined> {
  if (resolvedCli) {
    return resolvedCli;
  }
  const cfg = vscode.workspace.getConfiguration("javar");
  resolvedCli = await ensureJavarCli(cfg.get<string>("cliPath", "javar"), true);
  return resolvedCli;
}

/** Start javar-core sidecar only — never launches the user application. */
async function startSidecar(quiet = false): Promise<void> {
  if (coreProc && !coreProc.killed) {
    if (!quiet) {
      vscode.window.showInformationMessage("JavaR sidecar already running");
    }
    return;
  }
  const cli = await resolveCliPath();
  if (!cli) {
    return;
  }
  const cfg = vscode.workspace.getConfiguration("javar");
  const folder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? ".";
  const port = cfg.get<number>("agentPort", 19222);
  const projectName = vscode.workspace.workspaceFolders?.[0]?.name ?? "java-app";

  coreProc = spawn(cli, ["run", folder, "--watch-only", "--port", String(port)], {
    cwd: folder,
    shell: true,
    env: {
      ...process.env,
      JAVAR_AGENT_ADDR: `127.0.0.1:${port}`,
      JAVAR_PROJECT_NAME: projectName,
    },
  });
  coreProc.on("exit", () => {
    coreProc = undefined;
    statusBar.text = "$(flame) JavaR: Idle";
  });
  if (!quiet) {
    vscode.window.showInformationMessage(
      "JavaR sidecar started (watch-only). Run your app via IDE / mvn — use `javar enable --global` for invisible agent injection."
    );
  }
}

async function openDashboard(): Promise<void> {
  const cli = await resolveCliPath();
  if (!cli) {
    return;
  }
  const cfg = vscode.workspace.getConfiguration("javar");
  const port = cfg.get<number>("agentPort", 19222);
  const term = vscode.window.createTerminal({ name: "JavaR Control Center" });
  term.show();
  const quoted = cli.includes(" ") ? `"${cli}"` : cli;
  term.sendText(`${quoted} dashboard --addr 127.0.0.1:${port}`);
}

function requestTelemetry(host: string, port: number): Promise<Record<string, unknown>> {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host, port }, () => {
      socket.write(encodeFrame(7, Buffer.alloc(0)));
    });
    let buf = Buffer.alloc(0);
    socket.on("data", (chunk) => {
      buf = Buffer.concat([buf, chunk]);
      const decoded = tryDecode(buf);
      if (!decoded) {
        return;
      }
      socket.end();
      try {
        resolve(JSON.parse(decoded.payload.toString("utf8")));
      } catch (e) {
        reject(e);
      }
    });
    socket.on("error", reject);
    setTimeout(() => {
      socket.destroy();
      reject(new Error("timeout"));
    }, 2500);
  });
}

function sendHotDeploy(host: string, port: number, filePath: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const payload = Buffer.from(
      JSON.stringify({ state: "hot_deploy", detail: filePath }),
      "utf8"
    );
    const socket = net.createConnection({ host, port }, () => {
      socket.write(encodeFrame(8, payload));
    });
    socket.on("data", () => {
      socket.end();
      resolve();
    });
    socket.on("error", reject);
    setTimeout(() => {
      socket.destroy();
      resolve();
    }, 1500);
  });
}

const MAGIC = 0x4a415652;

function encodeFrame(kind: number, payload: Buffer): Buffer {
  const header = Buffer.alloc(10);
  header.writeUInt32LE(MAGIC, 0);
  header.writeUInt8(1, 4);
  header.writeUInt8(kind, 5);
  header.writeUInt32LE(payload.length, 6);
  return Buffer.concat([header, payload]);
}

function tryDecode(buf: Buffer): { payload: Buffer } | undefined {
  if (buf.length < 10) {
    return undefined;
  }
  const len = buf.readUInt32LE(6);
  if (buf.length < 10 + len) {
    return undefined;
  }
  return { payload: buf.subarray(10, 10 + len) };
}

class RegionsProvider implements vscode.TreeDataProvider<vscode.TreeItem> {
  private _onDidChange = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this._onDidChange.event;
  private regions = 0;
  private managed = 0;
  private project = "";

  update(regions: number, managed: number, project = ""): void {
    this.regions = regions;
    this.managed = managed;
    this.project = project;
    this._onDidChange.fire();
  }

  getTreeItem(el: vscode.TreeItem): vscode.TreeItem {
    return el;
  }

  getChildren(): Thenable<vscode.TreeItem[]> {
    const mb = (this.managed / (1024 * 1024)).toFixed(2);
    return Promise.resolve([
      new vscode.TreeItem(`Project: ${this.project || "(unknown)"}`),
      new vscode.TreeItem(`Managed regions: ${this.regions}`),
      new vscode.TreeItem(`Off-heap bytes: ${mb} MB`),
      new vscode.TreeItem("Mode: sidecar + telemetry (app via IDE / JAVA_TOOL_OPTIONS)"),
    ]);
  }
}
