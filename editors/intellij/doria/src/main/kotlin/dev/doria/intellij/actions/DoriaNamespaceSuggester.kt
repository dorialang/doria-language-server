package dev.doria.intellij.actions

import com.intellij.openapi.application.ReadAction
import com.intellij.openapi.project.DumbService
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VfsUtilCore
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.openapi.util.TextRange
import com.intellij.psi.PsiDirectory
import com.intellij.psi.search.FileTypeIndex
import com.intellij.psi.search.GlobalSearchScope
import dev.doria.intellij.DoriaFileType
import dev.doria.intellij.highlighting.DoriaLexer
import dev.doria.intellij.highlighting.DoriaTokenTypes

internal object DoriaNamespaceSuggester {
    fun suggest(project: Project, directory: PsiDirectory): List<String> {
        val configured = configuredNamespace(directory)
        if (DumbService.isDumb(project)) return listOfNotNull(configured)

        val indexed = ReadAction.compute<List<String>, RuntimeException> {
            indexedSuggestions(project, directory)
        }
        return listOfNotNull(configured) + indexed.filterNot { it == configured }
    }

    fun unambiguousSuggestion(project: Project, directory: PsiDirectory): String? {
        val configured = configuredNamespace(directory)
        if (configured != null || DumbService.isDumb(project)) return configured

        val indexed = ReadAction.compute<List<String>, RuntimeException> {
            indexedSuggestions(project, directory)
        }
        return chooseUnambiguous(configured, indexed)
    }

    fun configuredNamespace(directory: PsiDirectory): String? =
        DoriaAutoloadNamespaceResolver.suggest(directory.virtualFile)

    private fun indexedSuggestions(project: Project, directory: PsiDirectory): List<String> {
        val targetDirectory = pathSegments(directory.virtualFile)

        return FileTypeIndex.getFiles(DoriaFileType.INSTANCE, GlobalSearchScope.projectScope(project))
            .mapNotNull { file -> namespaceSource(file) }
            .mapNotNull { source ->
                inferNamespace(targetDirectory, source.directory, source.namespace)?.let { namespace ->
                    Suggestion(namespace, directoryDistance(targetDirectory, source.directory))
                }
            }
            .filter { it.namespace.isNotEmpty() }
            .groupBy(Suggestion::namespace)
            .map { (namespace, suggestions) ->
                Suggestion(namespace, suggestions.minOf(Suggestion::distance))
            }
            .sortedWith(compareBy(Suggestion::distance, Suggestion::namespace))
            .map(Suggestion::namespace)
    }

    private fun namespaceSource(file: VirtualFile): NamespaceSource? {
        val text = try {
            VfsUtilCore.loadText(file)
        } catch (_: Exception) {
            return null
        }
        val namespace = declaredNamespace(text) ?: return null
        return NamespaceSource(pathSegments(file.parent), namespace)
    }

    internal fun declaredNamespace(source: CharSequence): String? =
        namespaceDeclaration(source)?.name

    internal fun namespaceDeclaration(source: CharSequence): NamespaceDeclaration? {
        val lexer = DoriaLexer()
        lexer.start(source)
        var collecting = false
        var declarationStart = -1
        val segments = mutableListOf<String>()

        while (lexer.tokenType != null) {
            val text = source.subSequence(lexer.tokenStart, lexer.tokenEnd).toString()
            if (!collecting && lexer.tokenType == DoriaTokenTypes.KEYWORD && text == "namespace") {
                collecting = true
                declarationStart = lexer.tokenStart
            } else if (collecting && lexer.tokenType == DoriaTokenTypes.NAMESPACE_PATH) {
                segments += text
            } else if (collecting && text == ";") {
                val name = segments.takeIf { it.isNotEmpty() }?.joinToString("\\") ?: return null
                return NamespaceDeclaration(name, TextRange(declarationStart, lexer.tokenEnd))
            }
            lexer.advance()
        }

        return null
    }

    internal fun pathSegments(file: VirtualFile): List<String> =
        file.path.split('/').filter(String::isNotEmpty)

    internal fun inferNamespace(
        targetDirectory: List<String>,
        sourceDirectory: List<String>,
        declaredNamespace: String,
    ): String? {
        val namespaceSegments = declaredNamespace.split('\\')
        if (namespaceSegments.any { !DORIA_IDENTIFIER.matches(it) }) return null

        val commonLength = commonPrefixLength(targetDirectory, sourceDirectory)
        val sourceSuffix = sourceDirectory.drop(commonLength)
        if (sourceSuffix.size > namespaceSegments.size) return null
        if (namespaceSegments.takeLast(sourceSuffix.size) != sourceSuffix) return null

        val inferred = namespaceSegments.dropLast(sourceSuffix.size) + targetDirectory.drop(commonLength)
        return inferred.takeIf { it.all(DORIA_IDENTIFIER::matches) }?.joinToString("\\")
    }

    internal fun chooseUnambiguous(configured: String?, indexed: List<String>): String? =
        configured ?: indexed.distinct().singleOrNull()

    private fun directoryDistance(left: List<String>, right: List<String>): Int {
        val commonLength = commonPrefixLength(left, right)
        return left.size + right.size - (2 * commonLength)
    }

    private fun commonPrefixLength(left: List<String>, right: List<String>): Int {
        val limit = minOf(left.size, right.size)
        var index = 0
        while (index < limit && left[index] == right[index]) index++
        return index
    }

    private data class NamespaceSource(val directory: List<String>, val namespace: String)

    private data class Suggestion(val namespace: String, val distance: Int)

    internal data class NamespaceDeclaration(val name: String, val range: TextRange)

    private val DORIA_IDENTIFIER = Regex("[A-Za-z_][A-Za-z0-9_]*")
}
