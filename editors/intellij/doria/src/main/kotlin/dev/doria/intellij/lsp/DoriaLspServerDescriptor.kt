package dev.doria.intellij.lsp

import com.intellij.execution.configurations.GeneralCommandLine
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.ProjectWideLspServerDescriptor
import dev.doria.intellij.settings.DoriaSettings

class DoriaLspServerDescriptor(project: Project) : ProjectWideLspServerDescriptor(project, "Doria") {
    override fun isSupportedFile(file: VirtualFile): Boolean =
        DoriaLspFiles.isDoriaSourceFile(file)

    override fun createCommandLine(): GeneralCommandLine {
        val commandLine = GeneralCommandLine(DoriaLspServerPathResolver.resolve(project))
        project.basePath?.let { commandLine.withWorkDirectory(it) }
        DoriaSettings.getInstance().state.batonPath.trim().takeIf { it.isNotEmpty() }?.let {
            commandLine.withEnvironment(
                "DORIA_BATON_PATH",
                DoriaLspServerPathResolver.expandConfiguredPath(it, project.basePath),
            )
        }
        return commandLine
    }
}
