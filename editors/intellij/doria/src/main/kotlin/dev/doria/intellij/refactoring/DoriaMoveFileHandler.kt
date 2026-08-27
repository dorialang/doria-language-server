package dev.doria.intellij.refactoring

import com.intellij.openapi.util.Key
import com.intellij.psi.PsiDirectory
import com.intellij.psi.PsiDocumentManager
import com.intellij.psi.PsiElement
import com.intellij.psi.PsiFile
import com.intellij.refactoring.move.moveFilesOrDirectories.MoveFileHandler
import com.intellij.usageView.UsageInfo
import dev.doria.intellij.DoriaFileType
import dev.doria.intellij.actions.DoriaNamespaceSuggester

class DoriaMoveFileHandler : MoveFileHandler() {
    override fun canProcessElement(element: PsiFile): Boolean =
        element.fileType == DoriaFileType.INSTANCE

    override fun prepareMovedFile(
        file: PsiFile,
        moveDestination: PsiDirectory,
        oldToNewMap: MutableMap<PsiElement, PsiElement>,
    ) {
        val currentDirectory = file.virtualFile.parent ?: return
        val currentNamespace = DoriaNamespaceSuggester.declaredNamespace(file.text)
        val inferred = DoriaNamespaceSuggester.configuredNamespace(moveDestination)
            ?: currentNamespace?.let { namespace ->
            DoriaNamespaceSuggester.inferNamespace(
                    DoriaNamespaceSuggester.pathSegments(moveDestination.virtualFile),
                    DoriaNamespaceSuggester.pathSegments(currentDirectory),
                    namespace,
                )
            } ?: DoriaNamespaceSuggester.unambiguousSuggestion(file.project, moveDestination)

        if (inferred != null && inferred != currentNamespace) {
            file.putUserData(PENDING_NAMESPACE, inferred)
        }
    }

    override fun findUsages(
        file: PsiFile,
        newParent: PsiDirectory,
        searchInComments: Boolean,
        searchInNonJavaFiles: Boolean,
    ): List<UsageInfo> = emptyList()

    override fun retargetUsages(
        usageInfos: List<UsageInfo>,
        oldToNewMap: MutableMap<PsiElement, PsiElement>,
    ) = Unit

    override fun updateMovedFile(file: PsiFile) {
        val namespace = file.getUserData(PENDING_NAMESPACE) ?: return
        file.putUserData(PENDING_NAMESPACE, null)

        val documentManager = PsiDocumentManager.getInstance(file.project)
        val document = documentManager.getDocument(file) ?: return
        val updated = updateNamespace(document.text, namespace)
        if (updated == document.text) return

        document.setText(updated)
        documentManager.commitDocument(document)
    }

    internal companion object {
        private val PENDING_NAMESPACE = Key.create<String>("doria.pending.move.namespace")

        fun updateNamespace(source: String, namespace: String): String {
            val declaration = DoriaNamespaceSuggester.namespaceDeclaration(source)
            if (declaration == null) {
                return if (namespace.isEmpty()) source else "namespace $namespace;\n\n$source"
            }

            if (namespace.isNotEmpty()) {
                return source.replaceRange(declaration.range.startOffset, declaration.range.endOffset, "namespace $namespace;")
            }

            val end = trailingNamespaceWhitespaceEnd(source, declaration.range.endOffset)
            return source.removeRange(declaration.range.startOffset, end)
        }

        private fun trailingNamespaceWhitespaceEnd(source: String, offset: Int): Int = when {
            source.startsWith("\r\n\r\n", offset) -> offset + 4
            source.startsWith("\n\n", offset) -> offset + 2
            source.startsWith("\r\n", offset) -> offset + 2
            source.startsWith("\n", offset) -> offset + 1
            else -> offset
        }
    }
}
