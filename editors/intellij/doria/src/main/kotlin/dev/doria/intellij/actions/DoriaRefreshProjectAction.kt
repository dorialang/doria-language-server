package dev.doria.intellij.actions

import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.diagnostic.Logger
import com.intellij.platform.lsp.api.LspServer
import com.intellij.platform.lsp.api.LspServerManager
import dev.doria.intellij.lsp.DoriaLspServerSupportProvider
import org.eclipse.lsp4j.ExecuteCommandParams

class DoriaRefreshProjectAction : AnAction() {
    override fun update(event: AnActionEvent) {
        event.presentation.isEnabledAndVisible = event.project != null
    }

    override fun actionPerformed(event: AnActionEvent) {
        val project = event.project ?: return
        val provider = DoriaLspServerSupportProvider::class.java
        val manager = LspServerManager.getInstance(project)
        manager.startServersIfNeeded(provider)
        val servers = manager.getServersForProvider(provider)
        if (servers.isEmpty()) {
            return
        }

        ApplicationManager.getApplication().executeOnPooledThread {
            for (server in servers) {
                try {
                    server.sendRequestSync(LspServer.DEFAULT_REQUEST_TIMEOUT_MS) { languageServer ->
                        languageServer.workspaceService.executeCommand(
                            ExecuteCommandParams(COMMAND, emptyList<Any>()),
                        )
                    }
                } catch (error: RuntimeException) {
                    LOG.warn("Doria project refresh failed", error)
                }
            }
        }
    }

    companion object {
        internal const val COMMAND = "doria.refreshProject"
        private val LOG = Logger.getInstance(DoriaRefreshProjectAction::class.java)
    }
}
