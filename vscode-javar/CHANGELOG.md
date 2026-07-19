# Changelog

## 0.1.7

- Auto-install / repair JavaR CLI on activation (`JavaR: Install / Repair CLI`)
- Show running **project name** in the status bar and Off-Heap Regions view
- Activate on `pom.xml` / Gradle build files (Maven & Spring Boot workspaces)
- Align docs with Maven auto-bootstrap and Spring Boot fat-jar launch

## 0.1.6

- Document `javar setup` / `javar build` and embedded agent force-extract in Marketplace README
- Align install one-liners with root `scripts/install.ps1` / `install.sh`

## 0.1.5

- Fix Marketplace logo: use absolute GitHub raw URL (`vscode-javar/media/logo.png`) so Details page renders the image

## 0.1.4

- Full CLI command reference in Marketplace README (`init`, `run` options, `status`, `dashboard` / `tui`)
- Clarify Cockpit maps to `javar run --watch-only` vs smart `javar run` for the app JVM

## 0.1.3

- Cockpit auto-start uses `javar run --watch-only` (sidecar only; app launch is via smart CLI)
- Document smart `javar run`: Maven/Gradle detect, classes dir, main discovery, native lib path

## 0.1.2

- Document `javar run [PATH] -- [java args…]`: auto-discovers agent JAR, prepends absolute `-javaagent`, args after `--` go to `java`
- Clarify agent discovery (`--agent`, `JAVAR_AGENT_JAR`, relative target path)

## 0.1.1

- Add Marketplace README with full usage guide, commands, settings, and troubleshooting
- Include logo image in the extension package for the Details page

## 0.1.0

- Initial release: status bar telemetry, Force Re-sync, Start CLI, Control Center TUI, Off-Heap Regions view
