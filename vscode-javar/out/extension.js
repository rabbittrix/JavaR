"use strict";
var __createBinding = (this && this.__createBinding) || (Object.create ? (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    var desc = Object.getOwnPropertyDescriptor(m, k);
    if (!desc || ("get" in desc ? !m.__esModule : desc.writable || desc.configurable)) {
      desc = { enumerable: true, get: function() { return m[k]; } };
    }
    Object.defineProperty(o, k2, desc);
}) : (function(o, m, k, k2) {
    if (k2 === undefined) k2 = k;
    o[k2] = m[k];
}));
var __setModuleDefault = (this && this.__setModuleDefault) || (Object.create ? (function(o, v) {
    Object.defineProperty(o, "default", { enumerable: true, value: v });
}) : function(o, v) {
    o["default"] = v;
});
var __importStar = (this && this.__importStar) || (function () {
    var ownKeys = function(o) {
        ownKeys = Object.getOwnPropertyNames || function (o) {
            var ar = [];
            for (var k in o) if (Object.prototype.hasOwnProperty.call(o, k)) ar[ar.length] = k;
            return ar;
        };
        return ownKeys(o);
    };
    return function (mod) {
        if (mod && mod.__esModule) return mod;
        var result = {};
        if (mod != null) for (var k = ownKeys(mod), i = 0; i < k.length; i++) if (k[i] !== "default") __createBinding(result, mod, k[i]);
        __setModuleDefault(result, mod);
        return result;
    };
})();
Object.defineProperty(exports, "__esModule", { value: true });
exports.activate = activate;
exports.deactivate = deactivate;
const vscode = __importStar(require("vscode"));
const net = __importStar(require("net"));
const fs = __importStar(require("fs"));
const os = __importStar(require("os"));
const path = __importStar(require("path"));
const child_process_1 = require("child_process");
const ensureCli_1 = require("./ensureCli");
let statusBar;
let coreProc;
let pollTimer;
let lastTelemetry = { heap: 0, managed: 0, regions: 0, project: "" };
let resolvedCli;
/** Live agent port discovered from ~/.javar/agents (prefer user Spring apps). */
let resolvedAgentPort = 19222;
function activate(context) {
    statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
    statusBar.command = "javar.forceResync";
    statusBar.text = "$(flame) JavaR: Idle";
    statusBar.tooltip = "JavaR Cockpit — Force Re-sync (sidecar + telemetry only)";
    statusBar.show();
    context.subscriptions.push(statusBar);
    const regions = new RegionsProvider();
    vscode.window.registerTreeDataProvider("javar.regions", regions);
    context.subscriptions.push(vscode.commands.registerCommand("javar.connect", () => connectAndPoll(regions)), vscode.commands.registerCommand("javar.forceResync", () => forceResync()), vscode.commands.registerCommand("javar.startCli", () => startSidecar()), vscode.commands.registerCommand("javar.openDashboard", () => openDashboard()), vscode.commands.registerCommand("javar.installCli", () => installCli()));
    void bootstrap(regions);
}
async function bootstrap(regions) {
    const cfg = vscode.workspace.getConfiguration("javar");
    const configured = cfg.get("cliPath", "javar");
    const offer = cfg.get("autoInstallCli", true);
    resolvedCli = await (0, ensureCli_1.ensureJavarCli)(configured, offer);
    // Cockpit never launches the app JVM — only sidecar + telemetry.
    if (cfg.get("autoStart", true) && vscode.workspace.workspaceFolders?.length) {
        void connectAndPoll(regions);
        if (resolvedCli) {
            void startSidecar(true);
        }
    }
}
async function installCli() {
    const cfg = vscode.workspace.getConfiguration("javar");
    resolvedCli = undefined;
    resolvedCli = await (0, ensureCli_1.ensureJavarCli)(cfg.get("cliPath", "javar"), true);
}
function deactivate() {
    if (pollTimer) {
        clearInterval(pollTimer);
    }
    coreProc?.kill();
}
async function connectAndPoll(regions) {
    statusBar.text = "$(sync~spin) JavaR: Connecting…";
    if (pollTimer) {
        clearInterval(pollTimer);
    }
    pollTimer = setInterval(() => {
        void refreshTelemetry(regions);
    }, 2000);
    await refreshTelemetry(regions);
}
async function refreshTelemetry(regions) {
    const cfg = vscode.workspace.getConfiguration("javar");
    const host = cfg.get("agentHost", "127.0.0.1");
    const folder = vscode.workspace.workspaceFolders?.[0]?.name ?? "";
    resolvedAgentPort = resolveBestAgentPort(cfg.get("agentPort", 19222), folder);
    const port = resolvedAgentPort;
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
        const hist = Array.isArray(snap.reload_history) ? snap.reload_history.length : 0;
        statusBar.text = `$(flame) JavaR: Active${proj} · :${port} · Heap ${heapMb}MB · hist ${hist}`;
        regions.update(lastTelemetry.regions, lastTelemetry.managed, lastTelemetry.project);
    }
    catch {
        statusBar.text = "$(flame) JavaR: Watching (agent offline)";
    }
}
async function forceResync() {
    const editor = vscode.window.activeTextEditor;
    if (editor?.document.isDirty) {
        await editor.document.save();
    }
    const cfg = vscode.workspace.getConfiguration("javar");
    const host = cfg.get("agentHost", "127.0.0.1");
    const folder = vscode.workspace.workspaceFolders?.[0]?.name ?? "";
    resolvedAgentPort = resolveBestAgentPort(cfg.get("agentPort", 19222), folder);
    const port = resolvedAgentPort;
    const path = editor?.document.uri.fsPath ?? "";
    try {
        await sendHotDeploy(host, port, path);
        // Restart sidecar so it reconnects to the correct app port after save.
        if (coreProc && !coreProc.killed) {
            coreProc.kill();
            coreProc = undefined;
        }
        await startSidecar(true);
        vscode.window.setStatusBarMessage(`JavaR: Force Re-sync → :${port} (sidecar retargeted)`, 3000);
        statusBar.text = `$(sync) JavaR: Re-sync :${port}`;
    }
    catch (e) {
        vscode.window.showErrorMessage(`JavaR Force Re-sync failed: ${String(e)}`);
    }
}
async function resolveCliPath() {
    if (resolvedCli) {
        return resolvedCli;
    }
    const cfg = vscode.workspace.getConfiguration("javar");
    resolvedCli = await (0, ensureCli_1.ensureJavarCli)(cfg.get("cliPath", "javar"), true);
    return resolvedCli;
}
/** Start javar-core sidecar only — never launches the user application. */
async function startSidecar(quiet = false) {
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
    const projectName = vscode.workspace.workspaceFolders?.[0]?.name ?? "java-app";
    resolvedAgentPort = resolveBestAgentPort(cfg.get("agentPort", 19222), projectName);
    const port = resolvedAgentPort;
    const coreBin = resolveCoreBinary(cli);
    if (coreBin) {
        coreProc = (0, child_process_1.spawn)(coreBin, [folder], {
            cwd: folder,
            shell: false,
            env: {
                ...process.env,
                JAVAR_AGENT_ADDR: `127.0.0.1:${port}`,
                JAVAR_PROJECT_NAME: projectName,
            },
        });
    }
    else {
        coreProc = (0, child_process_1.spawn)(cli, ["run", folder, "--watch-only", "--port", String(port)], {
            cwd: folder,
            shell: true,
            env: {
                ...process.env,
                JAVAR_AGENT_ADDR: `127.0.0.1:${port}`,
                JAVAR_PROJECT_NAME: projectName,
            },
        });
    }
    coreProc.on("exit", () => {
        coreProc = undefined;
        statusBar.text = "$(flame) JavaR: Idle";
    });
    if (!quiet) {
        vscode.window.showInformationMessage(`JavaR sidecar watching → agent :${port}. Edit .java / .class to hot-reload.`);
    }
}
async function openDashboard() {
    const cli = await resolveCliPath();
    if (!cli) {
        return;
    }
    const cfg = vscode.workspace.getConfiguration("javar");
    const folder = vscode.workspace.workspaceFolders?.[0]?.name ?? "";
    resolvedAgentPort = resolveBestAgentPort(cfg.get("agentPort", 19222), folder);
    const port = resolvedAgentPort;
    const term = vscode.window.createTerminal({ name: "JavaR Control Center" });
    term.show();
    const quoted = cli.includes(" ") ? `"${cli}"` : cli;
    term.sendText(`${quoted} dashboard --addr 127.0.0.1:${port}`);
}
/** Pick DemoApplication / Spring app over Metals / language servers / Maven Launcher. */
function resolveBestAgentPort(fallback, workspaceName) {
    const home = process.env.JAVAR_HOME || path.join(os.homedir(), ".javar");
    const dir = path.join(home, "agents");
    if (!fs.existsSync(dir)) {
        return fallback;
    }
    const folder = (workspaceName || "").toLowerCase();
    const cands = [];
    for (const f of fs.readdirSync(dir)) {
        if (!f.endsWith(".json"))
            continue;
        try {
            const j = JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));
            const port = Number(j.port || 0);
            if (port < 19222 || port > 19232)
                continue;
            const name = String(j.name || j.project_name || "").toLowerCase();
            const cmd = String(j.cmd || "").toLowerCase();
            if (isTooling(name, cmd))
                continue;
            let score = 40;
            if (name.endsWith("application") || cmd.includes("application"))
                score += 200;
            if (name.includes("spring") || cmd.includes("springframework"))
                score += 120;
            if (name.includes("demo") || cmd.includes("demo"))
                score += 80;
            if (folder && (name.includes(folder) || cmd.includes(folder)))
                score += 150;
            if (name === "launcher" || cmd.includes("plexus") || cmd.includes("spring-boot:run")) {
                score -= 250;
            }
            cands.push({ port, score });
        }
        catch {
            /* ignore bad json */
        }
    }
    cands.sort((a, b) => b.score - a.score || a.port - b.port);
    return cands[0]?.port ?? fallback;
}
function isTooling(name, cmd) {
    const markers = [
        "eclipse",
        "equinox",
        "redhat.java",
        "jdt",
        "lemminx",
        "xmlserver",
        "languageserver",
        "language-server",
        "metals",
        "plexus",
        "classworlds.launcher",
        "spring-boot-language-server",
    ];
    if (markers.some((m) => name.includes(m) || cmd.includes(m)))
        return true;
    return name === "launcher";
}
function resolveCoreBinary(cliPath) {
    const exe = process.platform === "win32" ? "javar-core.exe" : "javar-core";
    const candidates = [
        path.join(os.homedir(), ".javar", "bin", exe),
        path.join(path.dirname(cliPath), exe),
    ];
    for (const c of candidates) {
        if (fs.existsSync(c))
            return c;
    }
    return undefined;
}
function requestTelemetry(host, port) {
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
            }
            catch (e) {
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
function sendHotDeploy(host, port, filePath) {
    return new Promise((resolve, reject) => {
        const payload = Buffer.from(JSON.stringify({ state: "hot_deploy", detail: filePath }), "utf8");
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
function encodeFrame(kind, payload) {
    const header = Buffer.alloc(10);
    header.writeUInt32LE(MAGIC, 0);
    header.writeUInt8(1, 4);
    header.writeUInt8(kind, 5);
    header.writeUInt32LE(payload.length, 6);
    return Buffer.concat([header, payload]);
}
function tryDecode(buf) {
    if (buf.length < 10) {
        return undefined;
    }
    const len = buf.readUInt32LE(6);
    if (buf.length < 10 + len) {
        return undefined;
    }
    return { payload: buf.subarray(10, 10 + len) };
}
class RegionsProvider {
    _onDidChange = new vscode.EventEmitter();
    onDidChangeTreeData = this._onDidChange.event;
    regions = 0;
    managed = 0;
    project = "";
    update(regions, managed, project = "") {
        this.regions = regions;
        this.managed = managed;
        this.project = project;
        this._onDidChange.fire();
    }
    getTreeItem(el) {
        return el;
    }
    getChildren() {
        const mb = (this.managed / (1024 * 1024)).toFixed(2);
        return Promise.resolve([
            new vscode.TreeItem(`Project: ${this.project || "(unknown)"}`),
            new vscode.TreeItem(`Managed regions: ${this.regions}`),
            new vscode.TreeItem(`Off-heap bytes: ${mb} MB`),
            new vscode.TreeItem("Mode: sidecar + telemetry (app via IDE / JAVA_TOOL_OPTIONS)"),
        ]);
    }
}
//# sourceMappingURL=extension.js.map