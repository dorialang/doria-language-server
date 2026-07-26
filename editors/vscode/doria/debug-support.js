"use strict";

const fs = require("fs");
const path = require("path");

function defaultDebugConfiguration() {
  return {
    type: "doria",
    request: "launch",
    name: "Run Doria project",
    mode: "project",
    cwd: "${workspaceFolder}",
    args: [],
    release: false,
    noDebug: true
  };
}

function buildRunArguments(configuration) {
  if (configuration.request !== "launch") {
    throw new Error(`Unsupported Doria debug request \`${configuration.request}\`.`);
  }
  const mode = configuration.mode ?? "project";
  if (!["project", "standalone"].includes(mode)) {
    throw new Error(`Unsupported Doria launch mode \`${mode}\`.`);
  }
  if (configuration.args !== undefined && !Array.isArray(configuration.args)) {
    throw new Error("Doria launch arguments must be an array of strings.");
  }

  const programArguments = configuration.args ?? [];
  if (programArguments.some((argument) => typeof argument !== "string")) {
    throw new Error("Every Doria launch argument must be a string.");
  }

  const arguments_ = ["run"];
  if (mode === "standalone") {
    if (typeof configuration.program !== "string" || configuration.program.trim().length === 0) {
      throw new Error("Standalone launch mode requires a Doria source file.");
    }
    arguments_.push(configuration.program);
  }
  if (configuration.release === true) {
    arguments_.push("--release");
  }
  if (programArguments.length > 0) {
    arguments_.push("--", ...programArguments);
  }
  return arguments_;
}

function findBatonProjectRoot(startPath, pathExists = fs.existsSync) {
  let directory = path.resolve(startPath);
  while (true) {
    if (pathExists(path.join(directory, "Baton.toml"))) {
      return directory;
    }
    const parent = path.dirname(directory);
    if (parent === directory) {
      return undefined;
    }
    directory = parent;
  }
}

module.exports = {
  buildRunArguments,
  defaultDebugConfiguration,
  findBatonProjectRoot
};
