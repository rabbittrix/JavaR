# Changelog

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
