package dev.doria.intellij.lsp

import com.google.gson.Gson
import com.google.gson.JsonElement
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.platform.lsp.api.LspServer
import com.intellij.platform.lsp.api.LspServerManager
import dev.doria.intellij.actions.DoriaSupertypeCandidates
import dev.doria.intellij.actions.DoriaSupertypeLookup
import dev.doria.intellij.actions.DoriaSupertypeSource
import org.eclipse.lsp4j.ExecuteCommandParams

/** Asks the Doria language server which types a new declaration may extend and implement. */
internal class DoriaLspSupertypeSource(private val project: Project) : DoriaSupertypeSource {
    override fun request(
        directory: VirtualFile,
        fileName: String,
        namespace: String,
        development: Boolean,
    ): DoriaSupertypeLookup {
        val provider = DoriaLspServerSupportProvider::class.java
        val manager = LspServerManager.getInstance(project)
        manager.startServersIfNeeded(provider)
        val server = manager.getServersForProvider(provider).firstOrNull()
            ?: return DoriaSupertypeLookup.Unavailable(NOT_RUNNING)
        // The server may run in another environment, such as WSL; its
        // descriptor maps the directory into the server's view.
        val directoryUri = server.descriptor.getFileUri(directory).trimEnd('/')
        val arguments = mapOf(
            "uri" to "$directoryUri/$fileName",
            "namespace" to namespace,
            "development" to development,
        )
        val response = try {
            server.sendRequestSync(LspServer.DEFAULT_REQUEST_TIMEOUT_MS) { languageServer ->
                languageServer.workspaceService.executeCommand(
                    ExecuteCommandParams(DoriaSupertypeSource.COMMAND, listOf(arguments)),
                )
            }
        } catch (error: RuntimeException) {
            return DoriaSupertypeLookup.Unavailable("The Doria language server failed to list types: ${error.message}")
        } ?: return DoriaSupertypeLookup.Unavailable(NO_ANSWER)
        val json = response as? JsonElement ?: Gson().toJsonTree(response)
        if (!json.isJsonObject) return DoriaSupertypeLookup.Unavailable(NO_ANSWER)
        return DoriaSupertypeLookup.Found(DoriaSupertypeCandidates.parse(json.asJsonObject))
    }

    private companion object {
        const val NOT_RUNNING =
            "The Doria language server is not running. Open a .doria file in this project to start it, " +
                "then reopen this dialog."
        const val NO_ANSWER =
            "The Doria language server did not answer. Update doria-lsp to a version that provides " +
                "${DoriaSupertypeSource.COMMAND}, or run Tools | Refresh Doria Project and reopen this dialog."
    }
}
