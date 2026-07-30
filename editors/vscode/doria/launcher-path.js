"use strict";

const fs = require("fs");
const os = require("os");
const path = require("path");

function resolveExecutablePath({
  configuredPath,
  environmentPath,
  workspaceRoot,
  executableName,
  windowsExecutableExtension = true,
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

  const executable = platform === "win32" && windowsExecutableExtension
    ? `${executableName}.exe`
    : executableName;
  const cargoExecutable = path.join(
    cargoInstallRoot || cargoHome || path.join(homeDirectory, ".cargo"),
    "bin",
    executable
  );
  if (pathExists(cargoExecutable)) {
    return { command: cargoExecutable, source: "Cargo install", rejectedPaths };
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
    executableName: "doriac"
  });
}

module.exports = {
  resolveBatonPath,
  resolveCompilerPath
};
