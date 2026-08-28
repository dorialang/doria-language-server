package dev.doria.intellij.documentation

import com.intellij.codeInsight.editorActions.enter.EnterHandlerDelegate
import com.intellij.codeInsight.editorActions.enter.EnterHandlerDelegateAdapter
import com.intellij.application.options.CodeStyle
import com.intellij.openapi.actionSystem.DataContext
import com.intellij.openapi.editor.Document
import com.intellij.openapi.editor.Editor
import com.intellij.openapi.editor.actionSystem.EditorActionHandler
import com.intellij.openapi.util.Ref
import com.intellij.platform.lsp.api.LspServerManager
import com.intellij.psi.PsiFile
import dev.doria.intellij.DoriaFileType
import dev.doria.intellij.lsp.DoriaLspServerSupportProvider
import org.eclipse.lsp4j.DocumentOnTypeFormattingParams
import org.eclipse.lsp4j.FormattingOptions
import org.eclipse.lsp4j.Position
import org.eclipse.lsp4j.TextEdit

class DoriaDocumentationEnterHandler : EnterHandlerDelegateAdapter() {
    override fun preprocessEnter(
        file: PsiFile,
        editor: Editor,
        caretOffset: Ref<Int>,
        caretAdvance: Ref<Int>,
        dataContext: DataContext,
        originalHandler: EditorActionHandler?,
    ): EnterHandlerDelegate.Result {
        if (file.fileType != DoriaFileType.INSTANCE || editor.isViewer) {
            return EnterHandlerDelegate.Result.Continue
        }
        val offset = caretOffset.get().coerceIn(0, editor.document.textLength)
        if (!isCommentOpenerAtCaret(editor.document, offset)) {
            return EnterHandlerDelegate.Result.Continue
        }

        val server = LspServerManager.getInstance(file.project)
            .getServersForProvider(DoriaLspServerSupportProvider::class.java)
            .firstOrNull { it.descriptor.isSupportedFile(file.virtualFile) }
            ?: return EnterHandlerDelegate.Result.Continue
        val indentOptions = CodeStyle.getIndentOptions(file)
        val params = DocumentOnTypeFormattingParams(
            server.getDocumentIdentifier(file.virtualFile),
            FormattingOptions(indentOptions.TAB_SIZE, !indentOptions.USE_TAB_CHARACTER),
            lspPosition(editor.document, offset),
            "\n",
        )
        val edits = try {
            server.sendRequestSync<List<TextEdit>>(REQUEST_TIMEOUT_MILLIS) { languageServer ->
                languageServer.textDocumentService.onTypeFormatting(params)
            }
        } catch (_: Exception) {
            null
        }.orEmpty()
        if (edits.isEmpty()) return EnterHandlerDelegate.Result.Continue

        val resolved = edits.mapNotNull { resolveEdit(editor.document, it) }
        if (resolved.isEmpty()) return EnterHandlerDelegate.Result.Continue
        val caretTarget = resolved.firstNotNullOfOrNull { edit ->
            summaryCaretOffset(edit, offset)
        }
        for (edit in resolved.sortedByDescending(ResolvedEdit::start)) {
            editor.document.replaceString(edit.start, edit.end, edit.text)
        }
        if (caretTarget != null) {
            editor.caretModel.moveToOffset(caretTarget)
            caretOffset.set(caretTarget)
        }
        return EnterHandlerDelegate.Result.Stop
    }

    private data class ResolvedEdit(
        val start: Int,
        val end: Int,
        val text: String,
    )

    private companion object {
        const val REQUEST_TIMEOUT_MILLIS = 750

        fun isCommentOpenerAtCaret(document: Document, offset: Int): Boolean {
            val line = document.getLineNumber(offset)
            val start = document.getLineStartOffset(line)
            return document.charsSequence.subSequence(start, offset).toString().trim() in setOf("/*", "/**")
        }

        fun lspPosition(document: Document, offset: Int): Position {
            val line = document.getLineNumber(offset)
            return Position(line, offset - document.getLineStartOffset(line))
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

        fun summaryCaretOffset(edit: ResolvedEdit, originalOffset: Int): Int? {
            if (originalOffset !in edit.start..edit.end) return null
            val marker = edit.text.indexOf(" * ", startIndex = edit.text.indexOf('\n').coerceAtLeast(0))
            return marker.takeIf { it >= 0 }?.let { edit.start + it + 3 }
        }
    }
}
