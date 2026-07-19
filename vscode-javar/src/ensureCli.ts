import * as vscode from "vscode";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { spawn } from "child_process";

const INSTALL_PS1 =
  "https://raw.githubusercontent.com/rabbittrix/JavaR/main/scripts/install.ps1";
const INSTALL_SH =
  "https://raw.githubusercontent.com/rabbittrix/JavaR/main/scripts/install.sh";

/** Resolve a usable `javar` binary, installing automatically when missing. */
export async function ensureJavarCli(
  configuredPath: string,
  offerInstall = true
): Promise<string | undefined> {
  const existing = await findJavar(configuredPath);
  if (existing) {
    return existing;
  }

  if (!offerInstall) {
    return undefined;
  }

  const choice = await vscode.window.showInformationMessage(
    "JavaR CLI not found. Install it now? (downloads into ~/.javar/bin and runs setup)",
    "Install",
    "Later"
  );
  if (choice !== "Install") {
    return undefined;
  }

  const ok = await vscode.window.withProgress(
    {
      location: vscode.ProgressLocation.Notification,
      title: "Installing JavaR CLI…",
      cancellable: false,
    },
    async () => runInstaller()
  );

  if (!ok) {
    vscode.window.showErrorMessage(
      "JavaR install failed. Run the install script from the README, then reload the window."
    );
    return undefined;
  }

  const after = await findJavar(configuredPath);
  if (after) {
    vscode.window.showInformationMessage(`JavaR ready: ${after}`);
  }
  return after;
}

async function findJavar(configured: string): Promise<string | undefined> {
  const candidates: string[] = [];
  if (configured && configured !== "javar") {
    candidates.push(configured);
  }
  const home = os.homedir();
  const exe = process.platform === "win32" ? "javar.exe" : "javar";
  candidates.push(path.join(home, ".javar", "bin", exe));
  candidates.push(path.join(home, ".cargo", "bin", exe));
  candidates.push(configured || "javar");

  for (const c of candidates) {
    if (c.includes(path.sep) || c.includes("/") || c.includes("\\")) {
      if (fs.existsSync(c)) {
        return c;
      }
    } else if (await commandExists(c)) {
      return c;
    }
  }
  return undefined;
}

function commandExists(cmd: string): Promise<boolean> {
  return new Promise((resolve) => {
    const whichCmd = process.platform === "win32" ? "where" : "which";
    const child = spawn(whichCmd, [cmd], { shell: true });
    child.on("close", (code) => resolve(code === 0));
    child.on("error", () => resolve(false));
  });
}

function runInstaller(): Promise<boolean> {
  return new Promise((resolve) => {
    let child;
    if (process.platform === "win32") {
      child = spawn(
        "powershell",
        [
          "-NoProfile",
          "-ExecutionPolicy",
          "Bypass",
          "-Command",
          `irm '${INSTALL_PS1}' | iex`,
        ],
        { shell: false }
      );
    } else {
      child = spawn("bash", ["-lc", `curl -fsSL '${INSTALL_SH}' | sh`], {
        shell: false,
      });
    }
    child.on("close", (code) => resolve(code === 0));
    child.on("error", () => resolve(false));
  });
}
