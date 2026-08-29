package dev.doria.intellij.actions

import com.intellij.codeInsight.intention.IntentionAction
import com.intellij.openapi.command.WriteCommandAction
import com.intellij.openapi.editor.Document
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.fileEditor.FileDocumentManager
import com.intellij.openapi.project.Project
import com.intellij.openapi.ui.popup.JBPopupFactory
import com.intellij.openapi.vfs.VirtualFileManager
import com.intellij.platform.lsp.api.LspServerManager
import com.intellij.psi.PsiDocumentManager
import com.intellij.psi.PsiFile
import com.intellij.ui.SimpleListCellRenderer
import com.intellij.util.IncorrectOperationException
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

abstract class DoriaLspCodeActionIntention : IntentionAction {
    protected abstract val chooserTitle: String
    protected abstract val commandName: String

    protected abstract fun accepts(action: CodeAction): Boolean

    override fun isAvailable(project: Project, editor: Editor, file: PsiFile): Boolean {
        if (file.fileType != DoriaFileType.INSTANCE || editor.isViewer) return false
        return requestActions(editor, file, AVAILABILITY_TIMEOUT_MILLIS).isNotEmpty()
    }

    @Throws(IncorrectOperationException::class)
    override fun invoke(project: Project, editor: Editor, file: PsiFile) {
        val actions = requestActions(editor, file, INVOCATION_TIMEOUT_MILLIS)
        when (actions.size) {
            0 -> return
            1 -> applyAction(project, file, actions.single())
            else -> showActionChooser(project, editor, file, actions)
        }
    }

    override fun startInWriteAction(): Boolean = false

    private fun requestActions(
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
            .filter(::accepts)
    }

    private fun applyAction(project: Project, file: PsiFile, action: CodeAction) {
        val edits = resolveEdits(action)
        if (edits.isEmpty()) return

        WriteCommandAction.runWriteCommandAction(project, commandName, null, {
            for ((document, documentEdits) in edits.groupBy(ResolvedEdit::document)) {
                for (edit in documentEdits.sortedByDescending(ResolvedEdit::start)) {
                    document.replaceString(edit.start, edit.end, edit.text)
                }
                PsiDocumentManager.getInstance(project).commitDocument(document)
            }
        }, file)
    }

    private fun resolveEdits(action: CodeAction): List<ResolvedEdit> {
        val changes = action.edit?.changes ?: return emptyList()
        val fileDocumentManager = FileDocumentManager.getInstance()
        val virtualFileManager = VirtualFileManager.getInstance()
        return changes.flatMap { (uri, edits) ->
            val virtualFile = virtualFileManager.findFileByUrl(uri) ?: return@flatMap emptyList()
            val document = fileDocumentManager.getDocument(virtualFile) ?: return@flatMap emptyList()
            edits.mapNotNull { edit -> resolveEdit(document, edit) }
        }
    }

    private fun showActionChooser(
        project: Project,
        editor: Editor,
        file: PsiFile,
        actions: List<CodeAction>,
    ) {
        JBPopupFactory.getInstance()
            .createPopupChooserBuilder(actions)
            .setTitle(chooserTitle)
            .setRenderer(SimpleListCellRenderer.create<CodeAction> { label, action, _ ->
                label.text = action.title
            })
            .setItemChosenCallback { action -> applyAction(project, file, action) }
            .createPopup()
            .showInBestPositionFor(editor)
    }

    private data class ResolvedEdit(
        val document: Document,
        val start: Int,
        val end: Int,
        val text: String,
    )

    private companion object {
        const val AVAILABILITY_TIMEOUT_MILLIS = 350
        const val INVOCATION_TIMEOUT_MILLIS = 1_500

        fun lspPosition(document: Document, offset: Int): Position {
            val safeOffset = offset.coerceIn(0, document.textLength)
            val line = document.getLineNumber(safeOffset)
            return Position(line, safeOffset - document.getLineStartOffset(line))
        }

        fun resolveEdit(document: Document, edit: TextEdit): ResolvedEdit? {
            val start = documentOffset(document, edit.range.start) ?: return null
            val end = documentOffset(document, edit.range.end) ?: return null
            if (end < start) return null
            return ResolvedEdit(document, start, end, edit.newText)
        }

        fun documentOffset(document: Document, position: Position): Int? {
            if (position.line !in 0 until document.lineCount) return null
            val start = document.getLineStartOffset(position.line)
            val end = document.getLineEndOffset(position.line)
            return (start + position.character).takeIf { it <= end }
        }
    }
}
