"use strict";

const cp = require("child_process");
const path = require("path");
const vscode = require("vscode");
const {
  resolveBatonPath,
  resolveCompilerPath
} = require("./launcher-path");
const {
  buildRunArguments,
  defaultDebugConfiguration,
  findBatonProjectRoot
} = require("./debug-support");
const { resolveServerPath } = require("./server-path");

let client;

function activate(context) {
  client = new DoriaLanguageClient(context);
  context.subscriptions.push(client);
  const debugConfigurationProvider = new DoriaDebugConfigurationProvider();
  const debugAdapterFactory = new DoriaDebugAdapterFactory();
  context.subscriptions.push(
    vscode.debug.registerDebugConfigurationProvider(
      "doria",
      debugConfigurationProvider
    ),
    vscode.debug.registerDebugAdapterDescriptorFactory(
      "doria",
      debugAdapterFactory
    )
  );
  client.start();
}

function deactivate() {
  if (client) {
    return client.dispose();
  }
}

class DoriaLanguageClient {
  constructor(context) {
    this.context = context;
    this.nextId = 1;
    this.pending = new Map();
    this.buffer = Buffer.alloc(0);
    this.diagnostics = vscode.languages.createDiagnosticCollection("doria");
    this.process = undefined;
    this.started = false;
    this.projectWatchers = [];
    this.dynamicProjectWatchers = new Map();

    context.subscriptions.push(this.diagnostics);
    context.subscriptions.push(
      vscode.commands.registerCommand("doria.refreshProject", () => this.refreshProject()),
      vscode.workspace.onDidChangeWorkspaceFolders((event) => this.didChangeWorkspaceFolders(event)),
      vscode.workspace.onDidOpenTextDocument((document) => this.didOpen(document)),
      vscode.workspace.onDidChangeTextDocument((event) => this.didChange(event)),
      vscode.workspace.onDidCloseTextDocument((document) => this.didClose(document)),
      vscode.workspace.onDidSaveTextDocument((document) => this.didSave(document)),
      vscode.languages.registerHoverProvider(
        { language: "doria" },
        {
          provideHover: (document, position) => this.provideHover(document, position)
        }
      ),
      vscode.languages.registerDefinitionProvider(
        { language: "doria" },
        {
          provideDefinition: (document, position) =>
            this.provideDefinition(document, position)
        }
      ),
      vscode.languages.registerReferenceProvider(
        { language: "doria" },
        {
          provideReferences: (document, position, context) =>
            this.provideReferences(document, position, context)
        }
      ),
      vscode.languages.registerRenameProvider(
        { language: "doria" },
        {
          provideRenameEdits: (document, position, newName) =>
            this.provideRenameEdits(document, position, newName)
        }
      ),
      vscode.languages.registerCompletionItemProvider(
        { language: "doria" },
        {
          provideCompletionItems: (document, position) => this.provideCompletionItems(document, position)
        },
        "$",
        ">",
        ":"
      ),
      vscode.languages.registerCodeActionsProvider(
        { language: "doria" },
        {
          provideCodeActions: (document, range) =>
            this.provideCodeActions(document, range)
        },
        {
          providedCodeActionKinds: [
            vscode.CodeActionKind.QuickFix,
            vscode.CodeActionKind.RefactorRewrite
          ]
        }
      ),
      vscode.languages.registerOnTypeFormattingEditProvider(
        { language: "doria" },
        {
          provideOnTypeFormattingEdits: (document, position, character, options) =>
            this.provideOnTypeFormattingEdits(document, position, character, options)
        },
        "\n"
      )
    );
    this.configureProjectWatchers();
  }

  start() {
    if (this.started) {
      return;
    }
    this.started = true;

    const resolution = resolveServerPath({
      configuredPath: vscode.workspace.getConfiguration("doria").get("languageServer.path"),
      environmentPath: process.env.DORIA_LSP_PATH,
      workspaceRoot: workspaceRoot(),
      extensionPath: this.context.extensionPath
    });
    const batonPath = vscode.workspace.getConfiguration("doria").get("baton.path")?.trim() ?? "";
    const child = cp.spawn(resolution.command, [], {
      cwd: workspaceRoot(),
      env: batonPath
        ? { ...process.env, DORIA_BATON_PATH: batonPath }
        : process.env,
      stdio: ["pipe", "pipe", "pipe"]
    });
    this.process = child;

    child.on("error", (error) => {
      const ignored = resolution.rejectedPaths.length === 0
        ? ""
        : ` Ignored missing ${resolution.rejectedPaths
          .map(({ source, path: rejectedPath }) => `${source} path ${rejectedPath}`)
          .join(" and ")}.`;
      vscode.window.showWarningMessage(
        `Doria language server failed to start from ${resolution.source}: ${error.message}.${ignored}`
      );
      this.resetServer(child, error);
    });
    child.stderr.on("data", (chunk) => {
      console.error(`[doria-lsp] ${chunk.toString()}`);
    });
    child.stdout.on("data", (chunk) => this.onData(chunk));
    child.on("close", (code, signal) => {
      this.resetServer(
        child,
        new Error(`Doria language server stopped (code ${code ?? "none"}, signal ${signal ?? "none"})`)
      );
    });

    this.sendRequest("initialize", {
      processId: process.pid,
      rootUri: vscode.workspace.workspaceFolders?.[0]?.uri.toString() ?? null,
      workspaceFolders: (vscode.workspace.workspaceFolders ?? []).map((folder) => ({
        uri: folder.uri.toString(),
        name: folder.name
      })),
      capabilities: {
        workspace: {
          didChangeWatchedFiles: {
            dynamicRegistration: true
          },
          workspaceFolders: true
        }
      },
      initializationOptions: {
        batonPath: batonPath || null
      }
    }).then(() => {
      this.sendNotification("initialized", {});
      for (const document of vscode.workspace.textDocuments) {
        this.didOpen(document);
      }
    }).catch(() => {
      // The spawn error path rejects the initialize request after surfacing a warning.
    });
  }

  dispose() {
    this.diagnostics.dispose();
    for (const watcher of this.projectWatchers) {
      watcher.dispose();
    }
    this.projectWatchers = [];
    this.disposeDynamicProjectWatchers();
    if (!this.process) {
      return Promise.resolve();
    }

    const child = this.process;
    return this.sendRequest("shutdown", {})
      .catch(() => undefined)
      .then(() => {
        this.sendNotification("exit", {});
        setTimeout(() => {
          if (!child.killed) {
            child.kill();
          }
        }, 1000);
      });
  }

  didOpen(document) {
    if (!isDoriaSource(document) || !this.process) {
      return;
    }

    this.sendNotification("textDocument/didOpen", {
      textDocument: {
        uri: document.uri.toString(),
        languageId: "doria",
        version: document.version,
        text: document.getText()
      }
    });
  }

  didChange(event) {
    if (!isDoriaSource(event.document) || !this.process) {
      return;
    }

    this.sendNotification("textDocument/didChange", {
      textDocument: {
        uri: event.document.uri.toString(),
        version: event.document.version
      },
      contentChanges: [
        {
          text: event.document.getText()
        }
      ]
    });
  }

  didSave(document) {
    if (!isDoriaSource(document) || !this.process) {
      return;
    }

    this.sendNotification("textDocument/didSave", {
      textDocument: {
        uri: document.uri.toString()
      }
    });
  }

  didClose(document) {
    if (!isDoria(document)) {
      return;
    }

    if (isDoriaSource(document) && this.process) {
      this.sendNotification("textDocument/didClose", {
        textDocument: {
          uri: document.uri.toString()
        }
      });
    }
    this.diagnostics.delete(document.uri);
  }

  didChangeWorkspaceFolders(event) {
    this.configureProjectWatchers();
    this.sendNotification("workspace/didChangeWorkspaceFolders", {
      event: {
        added: event.added.map((folder) => ({
          uri: folder.uri.toString(),
          name: folder.name
        })),
        removed: event.removed.map((folder) => ({
          uri: folder.uri.toString(),
          name: folder.name
        }))
      }
    });
  }

  configureProjectWatchers() {
    for (const watcher of this.projectWatchers) {
      watcher.dispose();
    }
    this.projectWatchers = [];
    const patterns = [
      "Baton.toml",
      "Baton.lock",
      "**/*.doria",
      ".doria/build/**",
      "build/.baton/**"
    ];
    for (const folder of vscode.workspace.workspaceFolders ?? []) {
      for (const pattern of patterns) {
        const watcher = vscode.workspace.createFileSystemWatcher(
          new vscode.RelativePattern(folder, pattern)
        );
        watcher.onDidCreate((uri) => this.didChangeWatchedFile(uri, 1));
        watcher.onDidChange((uri) => this.didChangeWatchedFile(uri, 2));
        watcher.onDidDelete((uri) => this.didChangeWatchedFile(uri, 3));
        this.projectWatchers.push(watcher);
      }
    }
  }

  didChangeWatchedFile(uri, type) {
    this.sendNotification("workspace/didChangeWatchedFiles", {
      changes: [{ uri: uri.toString(), type }]
    });
  }

  registerDynamicProjectWatchers(registrations) {
    for (const registration of registrations ?? []) {
      if (registration.method !== "workspace/didChangeWatchedFiles") {
        continue;
      }
      this.unregisterDynamicProjectWatchers([{ id: registration.id }]);
      const disposables = [];
      for (const entry of registration.registerOptions?.watchers ?? []) {
        const glob = entry.globPattern;
        if (!glob || typeof glob === "string" || !glob.baseUri || !glob.pattern) {
          continue;
        }
        const watcher = vscode.workspace.createFileSystemWatcher(
          new vscode.RelativePattern(vscode.Uri.parse(glob.baseUri), glob.pattern)
        );
        const kind = entry.kind ?? 7;
        if ((kind & 1) !== 0) {
          disposables.push(watcher.onDidCreate((uri) => this.didChangeWatchedFile(uri, 1)));
        }
        if ((kind & 2) !== 0) {
          disposables.push(watcher.onDidChange((uri) => this.didChangeWatchedFile(uri, 2)));
        }
        if ((kind & 4) !== 0) {
          disposables.push(watcher.onDidDelete((uri) => this.didChangeWatchedFile(uri, 3)));
        }
        disposables.push(watcher);
      }
      this.dynamicProjectWatchers.set(registration.id, disposables);
    }
  }

  unregisterDynamicProjectWatchers(unregistrations) {
    for (const registration of unregistrations ?? []) {
      const disposables = this.dynamicProjectWatchers.get(registration.id) ?? [];
      for (const disposable of disposables) {
        disposable.dispose();
      }
      this.dynamicProjectWatchers.delete(registration.id);
    }
  }

  disposeDynamicProjectWatchers() {
    this.unregisterDynamicProjectWatchers(
      [...this.dynamicProjectWatchers.keys()].map((id) => ({ id }))
    );
  }

  refreshProject() {
    if (!this.process) {
      return Promise.resolve();
    }
    return this.sendRequest("workspace/executeCommand", {
      command: "doria.refreshProject",
      arguments: []
    });
  }

  sendRequest(method, params) {
    if (!this.process) {
      return Promise.reject(new Error("Doria language server is not running"));
    }

    const id = this.nextId++;
    this.send({ jsonrpc: "2.0", id, method, params });
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
  }

  sendNotification(method, params) {
    if (!this.process) {
      return;
    }

    this.send({ jsonrpc: "2.0", method, params });
  }

  send(message) {
    if (!this.process) {
      return;
    }

    const body = Buffer.from(JSON.stringify(message), "utf8");
    const header = Buffer.from(`Content-Length: ${body.length}\r\n\r\n`, "ascii");
    this.process.stdin.write(Buffer.concat([header, body]));
  }

  resetServer(child, error) {
    if (child !== this.process) {
      return;
    }

    this.process = undefined;
    this.started = false;
    this.buffer = Buffer.alloc(0);
    this.diagnostics.clear();
    this.disposeDynamicProjectWatchers();
    this.rejectPending(error);
  }

  rejectPending(error) {
    for (const pending of this.pending.values()) {
      pending.reject(error);
    }
    this.pending.clear();
  }

  onData(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);

    while (true) {
      const headerEnd = this.buffer.indexOf("\r\n\r\n");
      if (headerEnd === -1) {
        return;
      }

      const header = this.buffer.slice(0, headerEnd).toString("ascii");
      const lengthMatch = header.match(/Content-Length:\s*(\d+)/i);
      if (!lengthMatch) {
        this.buffer = this.buffer.slice(headerEnd + 4);
        continue;
      }

      const length = Number(lengthMatch[1]);
      const messageEnd = headerEnd + 4 + length;
      if (this.buffer.length < messageEnd) {
        return;
      }

      const body = this.buffer.slice(headerEnd + 4, messageEnd).toString("utf8");
      this.buffer = this.buffer.slice(messageEnd);
      this.handleMessage(JSON.parse(body));
    }
  }

  handleMessage(message) {
    if (
      Object.prototype.hasOwnProperty.call(message, "id")
      && message.method === "client/registerCapability"
    ) {
      this.registerDynamicProjectWatchers(message.params?.registrations);
      this.send({ jsonrpc: "2.0", id: message.id, result: null });
      return;
    }
    if (
      Object.prototype.hasOwnProperty.call(message, "id")
      && message.method === "client/unregisterCapability"
    ) {
      this.unregisterDynamicProjectWatchers(
        message.params?.unregisterations ?? message.params?.unregistrations
      );
      this.send({ jsonrpc: "2.0", id: message.id, result: null });
      return;
    }
    if (Object.prototype.hasOwnProperty.call(message, "id")) {
      const pending = this.pending.get(message.id);
      if (pending) {
        this.pending.delete(message.id);
        if (message.error) {
          pending.reject(new Error(message.error.message));
        } else {
          pending.resolve(message.result);
        }
      }
      return;
    }

    if (message.method === "textDocument/publishDiagnostics") {
      this.publishDiagnostics(message.params);
    }
  }

  publishDiagnostics(params) {
    const uri = vscode.Uri.parse(params.uri);
    if (isEditorFixturePath(uri.fsPath)) {
      this.diagnostics.delete(uri);
      return;
    }

    const diagnostics = (params.diagnostics ?? []).map((diagnostic) => {
      const range = new vscode.Range(
        diagnostic.range.start.line,
        diagnostic.range.start.character,
        diagnostic.range.end.line,
        diagnostic.range.end.character
      );
      const item = new vscode.Diagnostic(
        range,
        diagnostic.message,
        toSeverity(diagnostic.severity)
      );
      item.code = diagnostic.code;
      item.source = diagnostic.source;
      return item;
    });
    this.diagnostics.set(uri, diagnostics);
  }

  provideHover(document, position) {
    if (!isDoriaSource(document) || !this.process) {
      return undefined;
    }

    return this.sendRequest("textDocument/hover", {
      textDocument: {
        uri: document.uri.toString()
      },
      position: toLspPosition(position)
    })
      .then((hover) => {
        if (!hover) {
          return undefined;
        }
        return new vscode.Hover(toHoverContents(hover.contents), hover.range ? toRange(hover.range) : undefined);
      })
      .catch(() => undefined);
  }

  provideDefinition(document, position) {
    if (!isDoriaSource(document) || !this.process) {
      return undefined;
    }

    return this.sendRequest("textDocument/definition", {
      textDocument: {
        uri: document.uri.toString()
      },
      position: toLspPosition(position)
    })
      .then((result) => {
        if (!result) {
          return undefined;
        }
        const locations = Array.isArray(result) ? result : [result];
        return locations.map(toLocation);
      })
      .catch(() => undefined);
  }

  provideRenameEdits(document, position, newName) {
    if (!isDoriaSource(document) || !this.process) {
      return undefined;
    }

    return this.sendRequest("textDocument/rename", {
      textDocument: {
        uri: document.uri.toString()
      },
      position: toLspPosition(position),
      newName
    })
      .then((edit) => edit ? toWorkspaceEdit(edit) : undefined)
      .catch(() => undefined);
  }

  provideReferences(document, position, context) {
    if (!isDoriaSource(document) || !this.process) {
      return undefined;
    }

    return this.sendRequest("textDocument/references", {
      textDocument: {
        uri: document.uri.toString()
      },
      position: toLspPosition(position),
      context: {
        includeDeclaration: context.includeDeclaration
      }
    })
      .then((locations) => (locations ?? []).map(toLocation))
      .catch(() => undefined);
  }

  provideCompletionItems(document, position) {
    if (!isDoriaSource(document) || !this.process) {
      return undefined;
    }

    return this.sendRequest("textDocument/completion", {
      textDocument: {
        uri: document.uri.toString()
      },
      position: toLspPosition(position)
    })
      .then((result) => {
        const items = Array.isArray(result) ? result : result?.items ?? [];
        return items.map((item) => {
          const completion = new vscode.CompletionItem(item.label, toCompletionKind(item.kind));
          completion.detail = item.detail;
          completion.documentation = item.documentation;
          completion.insertText = item.insertText;
          return completion;
        });
      })
      .catch(() => undefined);
  }

  provideCodeActions(document, range) {
    if (!isDoriaSource(document) || !this.process) {
      return undefined;
    }

    return this.sendRequest("textDocument/codeAction", {
      textDocument: {
        uri: document.uri.toString()
      },
      range: {
        start: toLspPosition(range.start),
        end: toLspPosition(range.end)
      },
      context: {
        diagnostics: []
      }
    })
      .then((actions) => (actions ?? []).map(toCodeAction))
      .catch(() => undefined);
  }

  provideOnTypeFormattingEdits(document, position, character, options) {
    if (!isDoriaSource(document) || !this.process || character !== "\n") {
      return undefined;
    }

    return this.sendRequest("textDocument/onTypeFormatting", {
      textDocument: {
        uri: document.uri.toString()
      },
      position: toLspPosition(position),
      ch: character,
      options: {
        tabSize: options.tabSize,
        insertSpaces: options.insertSpaces
      }
    })
      .then((edits) => (edits ?? []).map((edit) =>
        vscode.TextEdit.replace(toRange(edit.range), edit.newText)
      ))
      .catch(() => undefined);
  }
}

class DoriaDebugConfigurationProvider {
  provideDebugConfigurations() {
    return [defaultDebugConfiguration()];
  }

  async resolveDebugConfiguration(folder, requestedConfiguration) {
    const configuration = {
      ...defaultDebugConfiguration(),
      ...requestedConfiguration
    };
    const activeDocument = activeDoriaDocument();

    if (configuration.mode === "project") {
      const searchDirectories = [
        configuration.cwd && configuration.cwd !== "${workspaceFolder}"
          ? configuration.cwd
          : undefined,
        activeDocument ? path.dirname(activeDocument.uri.fsPath) : undefined,
        folder?.uri.fsPath
      ].filter(Boolean);
      const projectRoot = searchDirectories
        .map((directory) => findBatonProjectRoot(directory))
        .find(Boolean);
      if (!projectRoot) {
        vscode.window.showErrorMessage(
          "No Baton.toml was found for this workspace. Open a Doria project or use the standalone-file launch profile."
        );
        return undefined;
      }
      const dirtyDocuments = vscode.workspace.textDocuments.filter(
        (document) => isDoriaSource(document)
          && document.isDirty
          && isPathInside(projectRoot, document.uri.fsPath)
      );
      const saved = await Promise.all(
        dirtyDocuments.map((document) => document.save())
      );
      if (saved.some((didSave) => !didSave)) {
        vscode.window.showErrorMessage(
          "Save the Doria project files before launching the project."
        );
        return undefined;
      }
      configuration.cwd = projectRoot;
    } else if (!configuration.program || configuration.program === "${file}") {
      if (!activeDocument) {
        vscode.window.showErrorMessage(
          "Open a saved .doria file before launching standalone-file mode."
        );
        return undefined;
      }
      if (activeDocument.isDirty && !(await activeDocument.save())) {
        vscode.window.showErrorMessage(
          `Save ${activeDocument.fileName} before launching it.`
        );
        return undefined;
      }
      configuration.program = activeDocument.uri.fsPath;
    }

    if (configuration.mode !== "project" && !configuration.cwd) {
      configuration.cwd = folder?.uri.fsPath
        ?? path.dirname(configuration.program);
    } else if (
      configuration.mode !== "project"
      && configuration.cwd === "${workspaceFolder}"
      && !folder
    ) {
      configuration.cwd = path.dirname(configuration.program);
    }

    try {
      buildRunArguments(configuration);
    } catch (error) {
      vscode.window.showErrorMessage(error.message);
      return undefined;
    }
    return configuration;
  }

  async resolveDebugConfigurationWithSubstitutedVariables(_folder, configuration) {
    if (configuration.mode !== "standalone") {
      return configuration;
    }
    if (
      typeof configuration.program !== "string"
      || !configuration.program.toLowerCase().endsWith(".doria")
    ) {
      vscode.window.showErrorMessage(
        `Doria launch profiles require a .doria source file, got ${configuration.program}.`
      );
      return undefined;
    }

    try {
      await vscode.workspace.fs.stat(vscode.Uri.file(configuration.program));
    } catch (error) {
      vscode.window.showErrorMessage(
        `Doria source file does not exist: ${configuration.program}.`
      );
      return undefined;
    }
    return configuration;
  }
}

class DoriaDebugAdapterFactory {
  createDebugAdapterDescriptor(session) {
    const workspaceFolder = session.workspaceFolder;
    const configuration = vscode.workspace.getConfiguration(
      "doria",
      workspaceFolder?.uri
    );
    const resolution = session.configuration.mode === "standalone"
      ? resolveCompilerPath({
        configuredPath: configuration.get("compiler.path"),
        environmentPath: process.env.DORIAC_PATH,
        workspaceRoot: workspaceFolder?.uri.fsPath
      })
      : resolveBatonPath({
        configuredPath: configuration.get("baton.path"),
        environmentPath: process.env.DORIA_BATON_PATH,
        workspaceRoot: workspaceFolder?.uri.fsPath
      });
    return new vscode.DebugAdapterInlineImplementation(
      new DoriaDebugAdapter(session, resolution)
    );
  }
}

class DoriaDebugAdapter {
  constructor(session, compilerResolution) {
    this.session = session;
    this.compilerResolution = compilerResolution;
    this.sequence = 1;
    this.ended = false;
    this.execution = undefined;
    this.messageEmitter = new vscode.EventEmitter();
    this.onDidSendMessage = this.messageEmitter.event;
    this.taskEndSubscription = vscode.tasks.onDidEndTask((event) => {
      if (this.matchesExecution(event.execution)) {
        this.finish(undefined);
      }
    });
    this.taskProcessEndSubscription = vscode.tasks.onDidEndTaskProcess((event) => {
      if (this.matchesExecution(event.execution)) {
        this.finish(event.exitCode);
      }
    });
  }

  handleMessage(message) {
    if (message.type !== "request") {
      return;
    }

    switch (message.command) {
      case "initialize":
        this.respond(message, {
          supportsConfigurationDoneRequest: true,
          supportsTerminateRequest: true
        });
        this.event("initialized");
        break;
      case "launch":
        this.launch(message);
        break;
      case "setBreakpoints":
        this.respond(message, {
          breakpoints: (message.arguments?.breakpoints ?? []).map(() => ({
            verified: false,
            message: "Doria source-level breakpoints are not available yet."
          }))
        });
        break;
      case "setExceptionBreakpoints":
      case "configurationDone":
        this.respond(message);
        break;
      case "threads":
        this.respond(message, { threads: [] });
        break;
      case "terminate":
      case "disconnect":
        this.respond(message);
        this.execution?.terminate();
        this.finish(undefined);
        break;
      default:
        this.respond(message);
        break;
    }
  }

  async launch(request) {
    try {
      const configuration = request.arguments ?? {};
      const args = buildRunArguments(configuration);
      const scope = this.session.workspaceFolder ?? vscode.TaskScope.Global;
      const task = new vscode.Task(
        {
          type: "doria",
          debugSession: this.session.id
        },
        scope,
        configuration.name ?? "Run Doria program",
        "Doria",
        new vscode.ProcessExecution(
          this.compilerResolution.command,
          args,
          {
            cwd: configuration.cwd,
            env: configuration.env
          }
        ),
        []
      );
      task.presentationOptions = {
        clear: false,
        echo: true,
        focus: true,
        panel: vscode.TaskPanelKind.Dedicated,
        reveal: vscode.TaskRevealKind.Always
      };
      this.execution = await vscode.tasks.executeTask(task);
      this.respond(request);
    } catch (error) {
      const ignored = this.compilerResolution.rejectedPaths.length === 0
        ? ""
        : ` Ignored missing ${this.compilerResolution.rejectedPaths
          .map(({ source, path: rejectedPath }) => `${source} path ${rejectedPath}`)
          .join(" and ")}.`;
      this.respond(request, undefined, false, `${error.message}.${ignored}`);
      this.finish(1);
    }
  }

  matchesExecution(execution) {
    return execution.task.definition.debugSession === this.session.id;
  }

  respond(request, body, success = true, message) {
    this.send({
      type: "response",
      request_seq: request.seq,
      command: request.command,
      success,
      body,
      message
    });
  }

  event(event, body) {
    this.send({
      type: "event",
      event,
      body
    });
  }

  send(message) {
    this.messageEmitter.fire({
      seq: this.sequence++,
      ...message
    });
  }

  finish(exitCode) {
    if (this.ended) {
      return;
    }
    this.ended = true;
    if (exitCode !== undefined) {
      this.event("exited", { exitCode });
    }
    this.event("terminated");
  }

  dispose() {
    this.execution?.terminate();
    this.taskEndSubscription.dispose();
    this.taskProcessEndSubscription.dispose();
    this.messageEmitter.dispose();
  }
}

function workspaceRoot() {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

function activeDoriaDocument() {
  const document = vscode.window.activeTextEditor?.document;
  return document?.uri.scheme === "file" && isDoriaSource(document)
    ? document
    : undefined;
}

function isPathInside(directory, candidate) {
  const relative = path.relative(directory, candidate);
  return relative !== ""
    && relative !== ".."
    && !relative.startsWith(`..${path.sep}`)
    && !path.isAbsolute(relative);
}

function isDoria(document) {
  return document.languageId === "doria" || document.fileName.endsWith(".doria");
}

function isDoriaSource(document) {
  return isDoria(document) && !isEditorFixturePath(document.fileName);
}

function isEditorFixturePath(fileName) {
  return fileName.replace(/\\/g, "/").includes("/editors/fixtures/");
}

function toSeverity(severity) {
  switch (severity) {
    case 1:
      return vscode.DiagnosticSeverity.Error;
    case 2:
      return vscode.DiagnosticSeverity.Warning;
    case 3:
      return vscode.DiagnosticSeverity.Information;
    case 4:
      return vscode.DiagnosticSeverity.Hint;
    default:
      return vscode.DiagnosticSeverity.Error;
  }
}

function toLspPosition(position) {
  return {
    line: position.line,
    character: position.character
  };
}

function toRange(range) {
  return new vscode.Range(
    range.start.line,
    range.start.character,
    range.end.line,
    range.end.character
  );
}

function toLocation(location) {
  return new vscode.Location(
    vscode.Uri.parse(location.uri),
    toRange(location.range)
  );
}

function toWorkspaceEdit(workspaceEdit) {
  const edit = new vscode.WorkspaceEdit();
  for (const [uri, changes] of Object.entries(workspaceEdit.changes ?? {})) {
    edit.set(
      vscode.Uri.parse(uri),
      changes.map((change) =>
        vscode.TextEdit.replace(toRange(change.range), change.newText)
      )
    );
  }
  return edit;
}

function toHoverContents(contents) {
  if (typeof contents === "string") {
    return contents;
  }
  if (contents && contents.kind === "markdown") {
    return new vscode.MarkdownString(contents.value);
  }
  if (contents && contents.value) {
    return contents.value;
  }
  if (Array.isArray(contents)) {
    return contents.map(toHoverContents);
  }
  return "";
}

function toCompletionKind(kind) {
  switch (kind) {
    case 7:
      return vscode.CompletionItemKind.Class;
    case 14:
      return vscode.CompletionItemKind.Keyword;
    case 25:
      return vscode.CompletionItemKind.TypeParameter;
    default:
      return vscode.CompletionItemKind.Text;
  }
}

function toCodeAction(action) {
  const kind = {
    quickfix: vscode.CodeActionKind.QuickFix,
    "refactor.rewrite": vscode.CodeActionKind.RefactorRewrite
  }[action.kind];
  const result = new vscode.CodeAction(action.title, kind);
  result.isPreferred = action.isPreferred;

  if (action.edit?.changes) {
    result.edit = toWorkspaceEdit(action.edit);
  }
  return result;
}

module.exports = {
  activate,
  deactivate
};
