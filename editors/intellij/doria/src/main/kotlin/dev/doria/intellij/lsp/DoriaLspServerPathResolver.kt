package dev.doria.intellij.lsp

import com.intellij.openapi.project.Project
import com.intellij.openapi.util.SystemInfo
import dev.doria.intellij.settings.DoriaSettings
import java.nio.file.Path
import java.nio.file.Paths

object DoriaLspServerPathResolver {
    fun resolve(project: Project): String {
        val configured = DoriaSettings.getInstance().state.languageServerPath.trim()
        if (configured.isNotEmpty()) {
            return expandProjectPath(configured, project)
        }

        val fromEnvironment = System.getenv("DORIA_LSP_PATH")?.trim().orEmpty()
        if (fromEnvironment.isNotEmpty()) {
            return fromEnvironment
        }

        val installedBinary = cargoBinCandidates(
            System.getenv(),
            System.getProperty("user.home"),
        ).firstOrNull { it.toFile().exists() }
        if (installedBinary != null) {
            return installedBinary.toAbsolutePath().toString()
        }

        return executableName()
    }

    internal fun cargoBinCandidates(environment: Map<String, String>, userHome: String): List<Path> {
        val root = environment["CARGO_INSTALL_ROOT"]?.takeIf { it.isNotBlank() }
            ?: environment["CARGO_HOME"]?.takeIf { it.isNotBlank() }
            ?: Paths.get(userHome, ".cargo").toString()
        return listOf(Paths.get(root, "bin", executableName()))
    }

    private fun executableName(): String = if (SystemInfo.isWindows) "doria-lsp.exe" else "doria-lsp"

    private fun expandProjectPath(path: String, project: Project): String {
        var expanded = path
        val basePath = project.basePath
        if (basePath != null) {
            expanded = expanded.replace("\$PROJECT_DIR$", basePath)
        }
        if (expanded == "~" || expanded.startsWith("~/")) {
            expanded = System.getProperty("user.home") + expanded.removePrefix("~")
        }
        return expanded
    }
}
