"use strict";

const fs = require("fs");
const path = require("path");

function resolveServerPath({
  configuredPath,
  environmentPath,
  workspaceRoot,
  extensionPath,
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
  if (workspaceRoot) {
    const workspaceBinary = path.join(workspaceRoot, "target", "debug", executable);
    if (pathExists(workspaceBinary)) {
      return { command: workspaceBinary, source: "workspace", rejectedPaths };
    }
  }

  const bundledBinary = path.join(extensionPath, "bin", executable);
  if (pathExists(bundledBinary)) {
    return { command: bundledBinary, source: "bundled", rejectedPaths };
  }

  return { command: executable, source: "PATH", rejectedPaths };
}

module.exports = {
  resolveServerPath
};
