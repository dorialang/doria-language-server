package dev.doria.intellij.lsp

import com.intellij.ide.plugins.PluginManagerCore
import com.intellij.openapi.extensions.PluginId
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

        val bundledBinary = bundledCandidate(
            PluginManagerCore.getPlugin(PluginId.getId(PLUGIN_ID))?.pluginPath,
            System.getProperty("os.name"),
            System.getProperty("os.arch"),
        )
        if (bundledBinary != null && ensureExecutable(bundledBinary)) {
            return bundledBinary.toAbsolutePath().toString()
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

    internal fun bundledCandidate(pluginPath: Path?, osName: String, architecture: String): Path? {
        val platform = platformKey(osName, architecture) ?: return null
        val root = pluginPath ?: return null
        val executable = if (platform.startsWith("windows-")) "doria-lsp.exe" else "doria-lsp"
        return root.resolve("bin").resolve(platform).resolve(executable).takeIf { it.toFile().isFile }
    }

    internal fun platformKey(osName: String, architecture: String): String? {
        val operatingSystem = when {
            osName.contains("windows", ignoreCase = true) -> "windows"
            osName.contains("mac", ignoreCase = true) ||
                osName.contains("darwin", ignoreCase = true) -> "macos"
            osName.contains("linux", ignoreCase = true) -> "linux"
            else -> return null
        }
        val machine = when (architecture.lowercase()) {
            "amd64", "x86_64" -> "x86_64"
            "aarch64", "arm64" -> "aarch64"
            else -> return null
        }
        return "$operatingSystem-$machine"
    }

    private fun ensureExecutable(path: Path): Boolean =
        SystemInfo.isWindows || path.toFile().canExecute() || path.toFile().setExecutable(true, false)

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

    private const val PLUGIN_ID = "dev.doria.intellij"
}
