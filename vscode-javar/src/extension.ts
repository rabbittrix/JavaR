import * as vscode from "vscode";
import * as net from "net";
import { spawn, ChildProcess } from "child_process";

let statusBar: vscode.StatusBarItem;
let coreProc: ChildProcess | undefined;
let pollTimer: NodeJS.Timeout | undefined;
let lastTelemetry = { heap: 0, managed: 0, regions: 0 };

export function activate(context: vscode.ExtensionContext): void {
  statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
  statusBar.command = "javar.forceResync";
  statusBar.text = "$(flame) JavaR: Idle";
  statusBar.tooltip = "JavaR Cockpit — Force Re-sync";
  statusBar.show();
  context.subscriptions.push(statusBar);

  const regions = new RegionsProvider();
  vscode.window.registerTreeDataProvider("javar.regions", regions);

  context.subscriptions.push(
    vscode.commands.registerCommand("javar.connect", () => connectAndPoll(regions)),
    vscode.commands.registerCommand("javar.forceResync", () => forceResync()),
    vscode.commands.registerCommand("javar.startCli", () => startCli()),
    vscode.commands.registerCommand("javar.openDashboard", () => openDashboard())
  );

  const cfg = vscode.workspace.getConfiguration("javar");
  if (cfg.get<boolean>("autoStart", true) && vscode.workspace.workspaceFolders?.length) {
    void connectAndPoll(regions);
    void startCli(true);
  }
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
      heap: snap.java_heap_used ?? 0,
      managed: snap.javar_managed ?? 0,
      regions: snap.managed_regions ?? 0,
    };
    const heapMb = (lastTelemetry.heap / (1024 * 1024)).toFixed(1);
    const manMb = (lastTelemetry.managed / (1024 * 1024)).toFixed(1);
    statusBar.text = `$(flame) JavaR: Active · Heap ${heapMb}MB · Off-heap ${manMb}MB`;
    regions.update(lastTelemetry.regions, lastTelemetry.managed);
  } catch {
    statusBar.text = "$(flame) JavaR: Offline";
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

function startCli(quiet = false): void {
  if (coreProc && !coreProc.killed) {
    if (!quiet) {
      vscode.window.showInformationMessage("JavaR CLI already running");
    }
    return;
  }
  const cfg = vscode.workspace.getConfiguration("javar");
  const cli = cfg.get<string>("cliPath", "javar");
  const folder = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath ?? ".";
  const port = cfg.get<number>("agentPort", 19222);

  coreProc = spawn(cli, ["run", folder, "--watch-only", "--port", String(port)], {
    cwd: folder,
    shell: true,
    env: { ...process.env, JAVAR_AGENT_ADDR: `127.0.0.1:${port}` },
  });
  coreProc.on("exit", () => {
    coreProc = undefined;
    statusBar.text = "$(flame) JavaR: Idle";
  });
  if (!quiet) {
    vscode.window.showInformationMessage(`Started: ${cli} run`);
  }
}

function openDashboard(): void {
  const cfg = vscode.workspace.getConfiguration("javar");
  const cli = cfg.get<string>("cliPath", "javar");
  const port = cfg.get<number>("agentPort", 19222);
  const term = vscode.window.createTerminal({ name: "JavaR Control Center" });
  term.show();
  term.sendText(`${cli} dashboard --addr 127.0.0.1:${port}`);
}

function requestTelemetry(host: string, port: number): Promise<Record<string, number>> {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host, port }, () => {
      socket.write(encodeFrame(7, Buffer.alloc(0))); // Telemetry
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
      socket.write(encodeFrame(8, payload)); // HotDeploy
    });
    socket.on("data", () => {
      socket.end();
      resolve();
    });
    socket.on("error", reject);
    setTimeout(() => {
      socket.destroy();
      resolve(); // fire-and-forget ok
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

  update(regions: number, managed: number): void {
    this.regions = regions;
    this.managed = managed;
    this._onDidChange.fire();
  }

  getTreeItem(el: vscode.TreeItem): vscode.TreeItem {
    return el;
  }

  getChildren(): Thenable<vscode.TreeItem[]> {
    const mb = (this.managed / (1024 * 1024)).toFixed(2);
    return Promise.resolve([
      new vscode.TreeItem(`Managed regions: ${this.regions}`),
      new vscode.TreeItem(`Off-heap bytes: ${mb} MB`),
      new vscode.TreeItem("Backend: Panama / JNI (agent)"),
    ]);
  }
}
