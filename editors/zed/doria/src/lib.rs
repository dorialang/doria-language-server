use zed_extension_api::{
    self as zed, settings::LspSettings, Command, LanguageServerId, Result, Worktree,
};

struct DoriaExtension;

impl DoriaExtension {
    /// Resolve the `doria-lsp` executable and its arguments.
    ///
    /// Preference order, mirroring the VS Code extension:
    /// 1. an explicit `lsp.doria.binary.path` in the user's Zed settings;
    /// 2. `doria-lsp` on `PATH`.
    fn resolve_binary(
        &self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<(String, Vec<String>)> {
        if let Ok(settings) = LspSettings::for_worktree(language_server_id.as_ref(), worktree) {
            if let Some(binary) = settings.binary {
                if let Some(path) = binary.path {
                    return Ok((path, binary.arguments.unwrap_or_default()));
                }
            }
        }

        if let Some(path) = worktree.which("doria-lsp") {
            return Ok((path, Vec::new()));
        }

        Err(
            "doria-lsp was not found. Install it so it is on PATH \
             (`php scripts/build.php install-server`, or `cargo install --path server` \
             from the doria-language-server repository), or set `lsp.doria.binary.path` \
             in your Zed settings."
                .to_string(),
        )
    }
}

impl zed::Extension for DoriaExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        language_server_id: &LanguageServerId,
        worktree: &Worktree,
    ) -> Result<Command> {
        let (command, args) = self.resolve_binary(language_server_id, worktree)?;
        Ok(Command {
            command,
            args,
            env: worktree.shell_env(),
        })
    }
}

zed::register_extension!(DoriaExtension);
