package dev.doria.intellij.lsp

import com.intellij.execution.ExecutionException
import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.eel.provider.LocalEelDescriptor
import com.intellij.platform.eel.provider.getEelDescriptor
import com.intellij.platform.lsp.api.ProjectWideLspServerDescriptor
import dev.doria.intellij.settings.DoriaSettings

class DoriaLspServerDescriptor(project: Project) : ProjectWideLspServerDescriptor(project, "Doria") {
    override fun isSupportedFile(file: VirtualFile): Boolean =
        DoriaLspFiles.isDoriaSourceFile(file)

    override fun createCommandLine(): GeneralCommandLine {
        val server = DoriaLspServerPathResolver.resolve(project)
            ?: throw ExecutionException(MISSING_ENVIRONMENT_SERVER).also { notifyMissingServer() }
        val commandLine = GeneralCommandLine(server)
        project.basePath?.let { commandLine.withWorkDirectory(it) }
        // The Baton setting names a program on this machine. A project in
        // another environment runs Baton from that environment's PATH, where
        // the language server itself finds it.
        if (project.getEelDescriptor() == LocalEelDescriptor) {
            DoriaSettings.getInstance().state.batonPath.trim().takeIf { it.isNotEmpty() }?.let {
                commandLine.withEnvironment(
                    "DORIA_BATON_PATH",
                    DoriaLspServerPathResolver.expandConfiguredPath(it, project.basePath),
                )
            }
        }
        return commandLine
    }

    private fun notifyMissingServer() {
        NotificationGroupManager.getInstance()
            .getNotificationGroup(NOTIFICATION_GROUP)
            .createNotification("Doria language server not found", MISSING_ENVIRONMENT_SERVER, NotificationType.ERROR)
            .notify(project)
    }

    private companion object {
        const val NOTIFICATION_GROUP = "Doria"
        const val MISSING_ENVIRONMENT_SERVER =
            "This project is inside WSL (or another environment), and no doria-lsp is installed there. " +
                "Install the Doria tools inside that environment so ~/.cargo/bin/doria-lsp exists " +
                "(from a doria checkout there: php scripts/refresh_development_toolchain.php " +
                "--language-server <path to doria-language-server>), " +
                "or set Settings | Languages & Frameworks | Doria | Language server path to a doria-lsp " +
                "inside that environment, then restart the IDE."
    }
}
