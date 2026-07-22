/**
 * Workspace-scoped JavaR agent injection — NOT global JAVA_TOOL_OPTIONS.
 *
 * Enables:
 * - VS Code / Cursor "Run Java" / Debug  → java.debug.settings.vmArgs
 * - Integrated terminal `mvn` / `gradle` → JAVA_TOOL_OPTIONS + MAVEN_OPTS (this workspace only)
 * - `mvn spring-boot:run` → `.mvn/maven.config` (`spring-boot.run.agents` + port property)
 * - Sidecar pin hints via JAVAR_AGENT_ADDR
 *
 * Author: Roberto de Souza <rabbittrix@hotmail.com>
 */

import * as vscode from "vscode";
import * as fs from "fs";
import * as net from "net";
import * as os from "os";
import * as path from "path";

const MARKER = "javar-agent";

export interface InjectResult {
  agentJar: string;
  nativeLib: string;
  port: number;
  vmArgs: string;
  settingsPath?: string;
  mavenConfigPath?: string;
}

export function javarBinDir(): string {
  const home = process.env.JAVAR_HOME || path.join(os.homedir(), ".javar");
  return path.join(home, "bin");
}

export function resolveAgentAssets(): { agentJar: string; nativeLib: string } | undefined {
  const bin = javarBinDir();
  const agentJar = path.join(bin, "javar-agent.jar");
  if (!fs.existsSync(agentJar)) {
    return undefined;
  }
  const nativeName =
    process.platform === "win32"
      ? "javar_core.dll"
      : process.platform === "darwin"
        ? "libjavar_core.dylib"
        : "libjavar_core.so";
  const nativeLib = path.join(bin, nativeName);
  if (!fs.existsSync(nativeLib)) {
    return undefined;
  }
  return { agentJar, nativeLib };
}

/** Forward-slash paths for JVM flags (Windows-safe). */
export function forwardSlashes(p: string): string {
  return path
    .resolve(p)
    .replace(/\\/g, "/")
    .replace(/^\/\/\?\//, "");
}

export function buildVmArgs(port: number): string | undefined {
  const assets = resolveAgentAssets();
  if (!assets) {
    return undefined;
  }
  const agent = forwardSlashes(assets.agentJar);
  const native = forwardSlashes(assets.nativeLib);
  return `-javaagent:${agent}=port=${port} -Djavar.native.path=${native} -Djavar.launched.by=vscode`;
}

/**
 * First free agent port in 19222–19242 that is not already held by a tooling JVM
 * (Bloop / Metals / etc.). Sidecar pin must match the real app.
 */
export async function allocateAgentPort(preferred = 19222): Promise<number> {
  const start = preferred >= 19222 && preferred <= 19242 ? preferred : 19222;
  for (let p = start; p <= 19242; p++) {
    if (!(await portFree(p))) {
      // Occupied — only reuse if nothing answers JavaR telemetry, or it's free for bind.
      const tooling = await isToolingAgentPort(p);
      if (tooling) {
        continue;
      }
      // Something else listening but not JavaR tooling — try next.
      continue;
    }
    return p;
  }
  // Prefer a fresh free port from the top of the range if preferred block is crowded.
  for (let p = 19222; p <= 19242; p++) {
    if (await portFree(p)) {
      return p;
    }
  }
  return start;
}

function portFree(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const srv = net.createServer();
    srv.once("error", () => resolve(false));
    srv.once("listening", () => {
      srv.close(() => resolve(true));
    });
    srv.listen(port, "127.0.0.1");
  });
}

/** True when a JavaR agent on this port identifies as IDE/build noise. */
async function isToolingAgentPort(port: number): Promise<boolean> {
  try {
    const body = await tcpRequest("127.0.0.1", port, telemetryProbe(), 400);
    if (!body) {
      return false;
    }
    const name = String(body.project_name || "").toLowerCase();
    const cmd = String(body.jvm_cmd || "").toLowerCase();
    const markers = [
      "bloop",
      "bloopserver",
      "metals",
      "scala.meta",
      "languageserver",
      "language-server",
      "jdt.ls",
      "equinox",
      "plexus",
      "classworlds",
    ];
    return markers.some((m) => name.includes(m) || cmd.includes(m));
  } catch {
    return false;
  }
}

function telemetryProbe(): Buffer {
  // Minimal JavaR framed TELEMETRY request (magic JAVR) — same as extension poll.
  // Keep in sync with requestTelemetry in extension.ts if the wire format changes.
  const payload = Buffer.from("{}");
  const header = Buffer.alloc(10);
  header.writeUInt32BE(0x4a415652, 0); // JAVR
  header.writeUInt8(1, 4); // version
  header.writeUInt8(7, 5); // KIND_TELEMETRY
  header.writeUInt32BE(payload.length, 6);
  return Buffer.concat([header, payload]);
}

function tcpRequest(
  host: string,
  port: number,
  data: Buffer,
  timeoutMs: number
): Promise<Record<string, unknown> | undefined> {
  return new Promise((resolve) => {
    const sock = net.createConnection({ host, port }, () => {
      sock.write(data);
    });
    const chunks: Buffer[] = [];
    const timer = setTimeout(() => {
      sock.destroy();
      resolve(undefined);
    }, timeoutMs);
    sock.on("data", (c) => chunks.push(c));
    sock.on("error", () => {
      clearTimeout(timer);
      resolve(undefined);
    });
    sock.on("end", () => {
      clearTimeout(timer);
      try {
        const buf = Buffer.concat(chunks);
        if (buf.length < 10) {
          resolve(undefined);
          return;
        }
        const len = buf.readUInt32BE(6);
        const json = buf.subarray(10, 10 + len).toString("utf8");
        resolve(JSON.parse(json) as Record<string, unknown>);
      } catch {
        resolve(undefined);
      }
    });
  });
}

/**
 * Merge JavaR agent flags into workspace settings so Maven / Spring Boot / Run Java
 * pick up the agent without a user-global JAVA_TOOL_OPTIONS.
 */
export async function configureWorkspaceInjection(port: number): Promise<InjectResult | undefined> {
  const folder = vscode.workspace.workspaceFolders?.[0];
  if (!folder) {
    return undefined;
  }
  const vmArgs = buildVmArgs(port);
  const assets = resolveAgentAssets();
  if (!vmArgs || !assets) {
    return undefined;
  }

  const cfg = vscode.workspace.getConfiguration();
  const agentAddr = `127.0.0.1:${port}`;

  // 1) Debugger / Run Java / Spring Boot Dashboard launches
  await mergeVmArgsSetting("java.debug.settings.vmArgs", vmArgs);

  // 2) Integrated terminal only (does NOT touch HKCU / user env registry)
  const termEnvKey =
    process.platform === "win32"
      ? "terminal.integrated.env.windows"
      : process.platform === "darwin"
        ? "terminal.integrated.env.osx"
        : "terminal.integrated.env.linux";

  const existingTerm =
    (cfg.get<Record<string, string>>(termEnvKey) as Record<string, string> | undefined) ?? {};
  const nextTerm: Record<string, string> = { ...existingTerm };
  nextTerm.JAVA_TOOL_OPTIONS = mergeToolOptions(existingTerm.JAVA_TOOL_OPTIONS || "", vmArgs);
  nextTerm.MAVEN_OPTS = mergeToolOptions(existingTerm.MAVEN_OPTS || "", vmArgs);
  nextTerm.JAVAR_AGENT_ADDR = agentAddr;
  nextTerm.JAVAR_PINNED_ADDR = agentAddr;
  nextTerm.JAVAR_AGENT_PORT = String(port);
  nextTerm.JAVAR_NATIVE_PATH = forwardSlashes(assets.nativeLib);
  await cfg.update(termEnvKey, nextTerm, vscode.ConfigurationTarget.Workspace);

  // 3) spring-boot:run fork — reliable even when JAVA_TOOL_OPTIONS is missing
  const mavenConfigPath = writeMavenSpringBootInject(folder.uri.fsPath, assets.agentJar, port);

  const settingsPath = path.join(folder.uri.fsPath, ".vscode", "settings.json");
  return {
    agentJar: assets.agentJar,
    nativeLib: assets.nativeLib,
    port,
    vmArgs,
    settingsPath: fs.existsSync(settingsPath) ? settingsPath : undefined,
    mavenConfigPath,
  };
}

/** Write / refresh `.mvn/maven.config` so `mvn spring-boot:run` loads the agent on the forked JVM. */
export function writeMavenSpringBootInject(
  projectRoot: string,
  agentJar: string,
  port: number
): string | undefined {
  const pom = path.join(projectRoot, "pom.xml");
  if (!fs.existsSync(pom)) {
    return undefined;
  }
  const mvnDir = path.join(projectRoot, ".mvn");
  fs.mkdirSync(mvnDir, { recursive: true });
  const configPath = path.join(mvnDir, "maven.config");
  const agent = forwardSlashes(agentJar);
  // maven.config: one CLI arg per line. `#` comments are NOT portable — Maven
  // tokenizes them as unrecognized options.
  const agentsLine = `-Dspring-boot.run.agents=${agent}`;
  const jvmLine = `-Dspring-boot.run.jvmArguments=-Djavar.agent.port=${port}`;

  let existing = "";
  if (fs.existsSync(configPath)) {
    existing = fs.readFileSync(configPath, "utf8");
  }
  const next = upsertMavenJavarLines(existing, agentsLine, jvmLine);
  if (next !== existing) {
    fs.writeFileSync(configPath, next.endsWith("\n") ? next : `${next}\n`, "utf8");
  }
  return configPath;
}

function upsertMavenJavarLines(existing: string, agentsLine: string, jvmLine: string): string {
  const kept: string[] = [];
  for (const line of existing.split(/\r?\n/)) {
    const t = line.trim();
    if (!t) continue;
    if (
      t.startsWith("#") ||
      t.includes("javar-agent") ||
      t.includes("javar.agent.port") ||
      t.includes("spring-boot.run.agents") ||
      (t.includes("spring-boot.run.jvmArguments") && t.includes("javar"))
    ) {
      continue;
    }
    kept.push(t);
  }
  kept.push(agentsLine);
  kept.push(jvmLine);
  return kept.join("\n");
}

async function mergeVmArgsSetting(key: string, fragment: string): Promise<void> {
  const cfg = vscode.workspace.getConfiguration();
  const current = String(cfg.get<string>(key) || "");
  const next = mergeToolOptions(current, fragment);
  if (next !== current) {
    await cfg.update(key, next, vscode.ConfigurationTarget.Workspace);
  }
}

/** Keep non-JavaR tokens; replace prior JavaR agent tokens with `fragment`. */
export function mergeToolOptions(existing: string, fragment: string): string {
  const cleaned = existing
    .split(/\s+/)
    .filter((tok) => {
      if (!tok) return false;
      if (tok.includes(MARKER) || tok.startsWith("-Djavar.")) return false;
      return true;
    })
    .join(" ")
    .trim();
  if (!cleaned) {
    return fragment.trim();
  }
  return `${cleaned} ${fragment.trim()}`.trim();
}
