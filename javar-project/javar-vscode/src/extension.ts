import * as vscode from "vscode";
import * as net from "net";
import { JavaRClient, TelemetrySnapshot } from "./client";
import { TelemetryProvider } from "./telemetryView";

let client: JavaRClient | undefined;
let statusBar: vscode.StatusBarItem;
let telemetryProvider: TelemetryProvider;
let telemetryTimer: NodeJS.Timeout | undefined;

export function activate(context: vscode.ExtensionContext): void {
  telemetryProvider = new TelemetryProvider();
  vscode.window.registerTreeDataProvider("javar.telemetry", telemetryProvider);

  statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
  statusBar.command = "javar.showTelemetry";
  statusBar.text = "$(flame) JavaR: idle";
  statusBar.show();
  context.subscriptions.push(statusBar);

  context.subscriptions.push(
    vscode.commands.registerCommand("javar.connect", () => connect()),
    vscode.commands.registerCommand("javar.disconnect", () => disconnect()),
    vscode.commands.registerCommand("javar.hotDeploy", () => hotDeploy()),
    vscode.commands.registerCommand("javar.showTelemetry", () => showTelemetry())
  );

  // Auto-connect when a JavaR project is open.
  const folder = vscode.workspace.workspaceFolders?.[0];
  if (folder) {
    void connect().catch(() => {
      statusBar.text = "$(flame) JavaR: offline";
    });
  }
}

export function deactivate(): void {
  disconnect();
}

async function connect(): Promise<void> {
  const cfg = vscode.workspace.getConfiguration("javar");
  const host = cfg.get<string>("coreHost", "127.0.0.1");
  const port = cfg.get<number>("corePort", 19222);
  const interval = cfg.get<number>("telemetryIntervalMs", 2000);

  disconnect();
  client = new JavaRClient(host, port);
  await client.connect();
  statusBar.text = "$(flame) JavaR: connected";
  vscode.window.setStatusBarMessage(`JavaR connected to ${host}:${port}`, 2500);

  telemetryTimer = setInterval(() => {
    void refreshTelemetry();
  }, interval);
  await refreshTelemetry();
}

function disconnect(): void {
  if (telemetryTimer) {
    clearInterval(telemetryTimer);
    telemetryTimer = undefined;
  }
  client?.close();
  client = undefined;
  statusBar.text = "$(flame) JavaR: idle";
}

async function hotDeploy(): Promise<void> {
  if (!client?.isConnected()) {
    await connect();
  }
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    vscode.window.showWarningMessage("JavaR: no active editor");
    return;
  }
  const doc = editor.document;
  if (doc.isDirty) {
    await doc.save();
  }

  statusBar.text = "$(sync~spin) JavaR: hot deploy…";
  try {
    // HotDeploy asks the agent/core path; core watches files — this nudges an explicit deploy.
    await client!.hotDeploy(doc.uri.fsPath);
    statusBar.text = "$(flame) JavaR: deployed";
    vscode.window.showInformationMessage(`JavaR: Hot Deploy triggered for ${doc.fileName}`);
  } catch (err) {
    statusBar.text = "$(error) JavaR: deploy failed";
    vscode.window.showErrorMessage(`JavaR Hot Deploy failed: ${String(err)}`);
  }
}

async function refreshTelemetry(): Promise<void> {
  if (!client?.isConnected()) {
    return;
  }
  try {
    const snap = await client.telemetry();
    telemetryProvider.update(snap);
    const heapMb = (snap.java_heap_used / (1024 * 1024)).toFixed(1);
    const managedMb = (snap.javar_managed / (1024 * 1024)).toFixed(1);
    statusBar.text = `$(flame) Heap ${heapMb}MB | JavaR ${managedMb}MB`;
  } catch {
    statusBar.text = "$(flame) JavaR: connected";
  }
}

async function showTelemetry(): Promise<void> {
  await refreshTelemetry();
  await vscode.commands.executeCommand("javar.telemetry.focus");
}
