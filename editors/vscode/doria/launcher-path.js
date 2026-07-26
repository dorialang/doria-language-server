"use strict";

const fs = require("fs");
const path = require("path");

function resolveExecutablePath({
  configuredPath,
  environmentPath,
  workspaceRoot,
  executableName,
  workspaceCandidates = [],
  windowsExecutableExtension = true,
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

  const executable = platform === "win32" && windowsExecutableExtension
    ? `${executableName}.exe`
    : executableName;
  if (workspaceRoot && workspaceCandidates.length > 0) {
    for (const candidate of workspaceCandidates) {
      const workspaceExecutable = path.join(workspaceRoot, ...candidate, executable);
      if (pathExists(workspaceExecutable)) {
        return { command: workspaceExecutable, source: "workspace", rejectedPaths };
      }
    }
  }

  return { command: executable, source: "PATH", rejectedPaths };
}

function resolveBatonPath(options) {
  return resolveExecutablePath({
    ...options,
    executableName: "baton",
    windowsExecutableExtension: false
  });
}

function resolveCompilerPath(options) {
  return resolveExecutablePath({
    ...options,
    executableName: "doriac",
    workspaceCandidates: [["target", "debug"]]
  });
}

module.exports = {
  resolveBatonPath,
  resolveCompilerPath
};
