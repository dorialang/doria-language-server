package dev.doria.intellij.lsp

import com.intellij.ide.plugins.PluginManagerCore
import com.intellij.openapi.application.ApplicationInfo
import com.intellij.openapi.extensions.PluginId
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.BuildNumber
import com.intellij.openapi.util.SystemInfo
import com.intellij.platform.eel.EelDescriptor
import com.intellij.platform.eel.provider.LocalEelDescriptor
import com.intellij.platform.eel.provider.getEelDescriptor
import com.intellij.platform.eel.provider.utils.EelPathUtils
import dev.doria.intellij.settings.DoriaSettings
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths

object DoriaLspServerPathResolver {
    /**
     * The server to start for [project], as a path the IDE can run.
     *
     * A project inside WSL (or another non-local environment) is served by a
     * doria-lsp installed in that environment: from build 263 the platform
     * starts a language server there, translating an absolute path that lives
     * on the environment's machine, and translates file paths both ways. A
     * Windows binary cannot run there. Returns null when the environment has
     * no server; the caller reports how to install one.
     */
    fun resolve(project: Project): String? {
        val environment = project.getEelDescriptor()
        if (environment != LocalEelDescriptor &&
            platformRunsServersInProjectEnvironment(ApplicationInfo.getInstance().build)
        ) {
            return resolveInEnvironment(project, environment)
        }
        return resolveLocally(project)
    }

    /** From build 263 the LSP API starts servers inside a non-local project's environment. */
    internal fun platformRunsServersInProjectEnvironment(build: BuildNumber): Boolean =
        build.baselineVersion >= FIRST_BUILD_RUNNING_SERVERS_IN_PROJECT_ENVIRONMENT

    private fun resolveInEnvironment(project: Project, environment: EelDescriptor): String? {
        val configured = DoriaSettings.getInstance().state.languageServerPath.trim()
        if (configured.isNotEmpty()) {
            // The setting is application-wide, so it serves this project only
            // when it names a server on the project's own machine.
            val path = Paths.get(expandConfiguredPath(configured, project.basePath))
            if (path.isAbsolute && path.getEelDescriptor() == environment) {
                return path.toString()
            }
        }
        val home = EelPathUtils.getHomePath(environment)
        return cargoBinCandidates(emptyMap(), home.toString(), POSIX_EXECUTABLE)
            .firstOrNull { Files.isRegularFile(it) }
            ?.toString()
    }

    private fun resolveLocally(project: Project): String {
        val configured = DoriaSettings.getInstance().state.languageServerPath.trim()
        if (configured.isNotEmpty()) {
            return expandConfiguredPath(configured, project.basePath)
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
            executableName(),
        ).firstOrNull { it.toFile().exists() }
        if (installedBinary != null) {
            return installedBinary.toAbsolutePath().toString()
        }

        return executableName()
    }

    internal fun cargoBinCandidates(
        environment: Map<String, String>,
        userHome: String,
        executable: String,
    ): List<Path> {
        val root = environment["CARGO_INSTALL_ROOT"]?.takeIf { it.isNotBlank() }
            ?: environment["CARGO_HOME"]?.takeIf { it.isNotBlank() }
            ?: Paths.get(userHome, ".cargo").toString()
        return listOf(Paths.get(root, "bin", executable))
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

    internal fun expandConfiguredPath(
        path: String,
        projectBasePath: String?,
        userHome: String = System.getProperty("user.home"),
    ): String {
        var expanded = path
        if (projectBasePath != null) {
            expanded = expanded.replace("\$PROJECT_DIR$", projectBasePath)
        }
        if (expanded == "~" || expanded.startsWith("~/")) {
            expanded = userHome + expanded.removePrefix("~")
        }
        return expanded
    }

    private const val PLUGIN_ID = "dev.doria.intellij"
    private const val POSIX_EXECUTABLE = "doria-lsp"
    private const val FIRST_BUILD_RUNNING_SERVERS_IN_PROJECT_ENVIRONMENT = 263
}
