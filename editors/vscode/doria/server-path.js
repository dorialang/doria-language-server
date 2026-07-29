"use strict";

const fs = require("fs");
const os = require("os");
const path = require("path");

function resolveServerPath({
  configuredPath,
  environmentPath,
  workspaceRoot,
  extensionPath,
  cargoInstallRoot = process.env.CARGO_INSTALL_ROOT,
  cargoHome = process.env.CARGO_HOME,
  homeDirectory = os.homedir(),
  platform = process.platform,
  pathExists = fs.existsSync
}) {
  const rejectedPaths = [];

  for (const [source, candidate] of [
    ["setting", configuredPath],
    ["environment", environmentPath]
  ]) {
    if (!candidate || candidate.trim().length === 0) {
      continue;
    }

    const requested = candidate.trim();
    const resolved = path.isAbsolute(requested)
      ? requested
      : path.resolve(workspaceRoot || process.cwd(), requested);
    if (pathExists(resolved)) {
      return { command: resolved, source, rejectedPaths };
    }
    rejectedPaths.push({ source, path: resolved });
  }

  const executable = platform === "win32" ? "doria-lsp.exe" : "doria-lsp";
  const bundledBinary = path.join(extensionPath, "bin", executable);
  if (pathExists(bundledBinary)) {
    return { command: bundledBinary, source: "bundled", rejectedPaths };
  }

  const cargoBinary = path.join(
    cargoInstallRoot || cargoHome || path.join(homeDirectory, ".cargo"),
    "bin",
    executable
  );
  if (pathExists(cargoBinary)) {
    return { command: cargoBinary, source: "Cargo install", rejectedPaths };
  }

  return { command: executable, source: "PATH", rejectedPaths };
}

module.exports = {
  resolveServerPath
};
