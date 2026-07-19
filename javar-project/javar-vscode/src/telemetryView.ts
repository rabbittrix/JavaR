import * as vscode from "vscode";
import { TelemetrySnapshot } from "./client";

export class TelemetryProvider implements vscode.TreeDataProvider<TelemetryItem> {
  private _onDidChangeTreeData = new vscode.EventEmitter<TelemetryItem | undefined | void>();
  readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

  private snapshot: TelemetrySnapshot = {
    java_heap_used: 0,
    java_heap_max: 0,
    javar_managed: 0,
    reload_count: 0,
  };

  update(snap: TelemetrySnapshot): void {
    this.snapshot = snap;
    this._onDidChangeTreeData.fire();
  }

  getTreeItem(element: TelemetryItem): vscode.TreeItem {
    return element;
  }

  getChildren(): Thenable<TelemetryItem[]> {
    const s = this.snapshot;
    const fmt = (n: number) => `${(n / (1024 * 1024)).toFixed(2)} MB`;
    return Promise.resolve([
      new TelemetryItem("Java Heap Used", fmt(s.java_heap_used)),
      new TelemetryItem("Java Heap Max", fmt(s.java_heap_max)),
      new TelemetryItem("JavaR Managed Memory", fmt(s.javar_managed)),
      new TelemetryItem("Reload Count", String(s.reload_count)),
      new TelemetryItem("Loaded Classes", String(s.loaded_classes ?? "—")),
    ]);
  }
}

class TelemetryItem extends vscode.TreeItem {
  constructor(label: string, value: string) {
    super(label, vscode.TreeItemCollapsibleState.None);
    this.description = value;
  }
}
