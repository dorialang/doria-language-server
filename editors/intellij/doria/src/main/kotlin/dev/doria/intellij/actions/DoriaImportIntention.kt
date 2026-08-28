package dev.doria.intellij.actions

import com.intellij.codeInsight.intention.IntentionAction
import com.intellij.openapi.command.WriteCommandAction
import com.intellij.openapi.editor.Document
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.project.Project
import com.intellij.openapi.ui.popup.JBPopupFactory
import com.intellij.psi.PsiDocumentManager
import com.intellij.psi.PsiFile
import com.intellij.ui.SimpleListCellRenderer
import com.intellij.util.IncorrectOperationException
import com.intellij.platform.lsp.api.LspServerManager
import dev.doria.intellij.DoriaFileType
import dev.doria.intellij.lsp.DoriaLspServerSupportProvider
import org.eclipse.lsp4j.CodeAction
import org.eclipse.lsp4j.CodeActionContext
import org.eclipse.lsp4j.CodeActionParams
import org.eclipse.lsp4j.Command
import org.eclipse.lsp4j.Diagnostic
import org.eclipse.lsp4j.Position
import org.eclipse.lsp4j.Range
import org.eclipse.lsp4j.TextEdit
import org.eclipse.lsp4j.jsonrpc.messages.Either

class DoriaImportIntention : IntentionAction {
    override fun getText(): String = "Use import"

    override fun getFamilyName(): String = "Doria imports"

    override fun isAvailable(project: Project, editor: Editor, file: PsiFile): Boolean {
        if (file.fileType != DoriaFileType.INSTANCE || editor.isViewer) return false
        return requestImportActions(editor, file, AVAILABILITY_TIMEOUT_MILLIS).isNotEmpty()
    }

    @Throws(IncorrectOperationException::class)
    override fun invoke(project: Project, editor: Editor, file: PsiFile) {
        val actions = requestImportActions(editor, file, INVOCATION_TIMEOUT_MILLIS)
        when (actions.size) {
            0 -> return
            1 -> applyAction(project, editor, file, actions.single())
            else -> showActionChooser(project, editor, file, actions)
        }
    }

    private fun applyAction(project: Project, editor: Editor, file: PsiFile, action: CodeAction) {
        val edits = currentDocumentEdits(editor.document, file.virtualFile.url, action)
        if (edits.isEmpty()) return

        WriteCommandAction.runWriteCommandAction(project, "Use Doria import", null, {
            for (edit in edits.sortedByDescending(ResolvedEdit::start)) {
                editor.document.replaceString(edit.start, edit.end, edit.text)
            }
            PsiDocumentManager.getInstance(project).commitDocument(editor.document)
        }, file)
    }

    override fun startInWriteAction(): Boolean = false

    private fun requestImportActions(
        editor: Editor,
        file: PsiFile,
        timeoutMillis: Int,
    ): List<CodeAction> {
        val server = LspServerManager.getInstance(file.project)
            .getServersForProvider(DoriaLspServerSupportProvider::class.java)
            .firstOrNull { it.descriptor.isSupportedFile(file.virtualFile) }
            ?: return emptyList()
        val position = lspPosition(editor.document, editor.caretModel.offset)
        val params = CodeActionParams(
            server.getDocumentIdentifier(file.virtualFile),
            Range(position, position),
            CodeActionContext(emptyList<Diagnostic>()),
        )
        return try {
            server.sendRequestSync<List<Either<Command, CodeAction>>>(timeoutMillis) { languageServer ->
                languageServer.textDocumentService.codeAction(params)
            }
        } catch (_: Exception) {
            null
        }.orEmpty()
            .filter(Either<Command, CodeAction>::isRight)
            .map(Either<Command, CodeAction>::getRight)
            .filter { it.title.startsWith(IMPORT_ACTION_PREFIX) }
    }

    private fun showActionChooser(
        project: Project,
        editor: Editor,
        file: PsiFile,
        actions: List<CodeAction>,
    ) {
        JBPopupFactory.getInstance()
            .createPopupChooserBuilder(actions)
            .setTitle("Choose the declaration to import")
            .setRenderer(SimpleListCellRenderer.create<CodeAction> { label, action, _ ->
                label.text = action.title
            })
            .setItemChosenCallback { action -> applyAction(project, editor, file, action) }
            .createPopup()
            .showInBestPositionFor(editor)
    }

    private data class ResolvedEdit(
        val start: Int,
        val end: Int,
        val text: String,
    )

    private companion object {
        const val AVAILABILITY_TIMEOUT_MILLIS = 350
        const val INVOCATION_TIMEOUT_MILLIS = 1_500
        const val IMPORT_ACTION_PREFIX = "Use import for "

        fun lspPosition(document: Document, offset: Int): Position {
            val safeOffset = offset.coerceIn(0, document.textLength)
            val line = document.getLineNumber(safeOffset)
            return Position(line, safeOffset - document.getLineStartOffset(line))
        }

        fun currentDocumentEdits(
            document: Document,
            uri: String,
            action: CodeAction,
        ): List<ResolvedEdit> {
            val changes = action.edit?.changes ?: return emptyList()
            val edits = changes[uri] ?: changes.values.singleOrNull() ?: return emptyList()
            return edits.mapNotNull { edit ->
                resolveEdit(document, edit)
            }
        }

        fun resolveEdit(document: Document, edit: TextEdit): ResolvedEdit? {
            val start = documentOffset(document, edit.range.start) ?: return null
            val end = documentOffset(document, edit.range.end) ?: return null
            if (end < start) return null
            return ResolvedEdit(start, end, edit.newText)
        }

        fun documentOffset(document: Document, position: Position): Int? {
            if (position.line !in 0 until document.lineCount) return null
            val start = document.getLineStartOffset(position.line)
            val end = document.getLineEndOffset(position.line)
            return (start + position.character).takeIf { it <= end }
        }
    }
}
