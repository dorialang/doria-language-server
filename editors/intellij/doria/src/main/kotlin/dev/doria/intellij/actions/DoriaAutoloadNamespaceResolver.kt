package dev.doria.intellij.actions

import com.intellij.openapi.vfs.VfsUtilCore
import com.intellij.openapi.vfs.VirtualFile
import org.tomlj.Toml
import org.tomlj.TomlTable

internal object DoriaAutoloadNamespaceResolver {
    fun suggest(directory: VirtualFile): String? = placement(directory)?.namespace

    /** The namespace and autoload scope Baton gives sources in `directory`. */
    fun placement(directory: VirtualFile): SourcePlacement? {
        val manifest = nearestManifest(directory) ?: return null
        val source = try {
            VfsUtilCore.loadText(manifest)
        } catch (_: Exception) {
            return null
        }

        return inferPlacement(
            pathSegments(manifest.parent),
            pathSegments(directory),
            mappings(source),
        )
    }

    private fun nearestManifest(directory: VirtualFile): VirtualFile? {
        var current: VirtualFile? = directory
        while (current != null) {
            current.findChild(MANIFEST_NAME)?.takeIf { !it.isDirectory }?.let { return it }
            current = current.parent
        }
        return null
    }

    internal fun mappings(source: String): List<AutoloadMapping> {
        val document = Toml.parse(source)
        if (document.hasErrors()) return emptyList()

        return TABLE_NAMES.flatMap { (section, development) ->
            val sectionTable = document.get(section) as? TomlTable
            val namespaceTable = sectionTable?.get(NAMESPACES) as? TomlTable
            namespaceTable?.let { readMappings(it, development) }.orEmpty()
        }
    }

    private fun readMappings(table: TomlTable, development: Boolean): List<AutoloadMapping> =
        table.keySet().mapNotNull { prefix ->
            val path = when (val value = table.get(listOf(prefix))) {
                is String -> value
                is TomlTable -> value.get("path") as? String
                else -> null
            } ?: return@mapNotNull null

            val namespace = namespaceSegments(prefix) ?: return@mapNotNull null
            val directory = mappingPathSegments(path) ?: return@mapNotNull null
            AutoloadMapping(namespace, directory, development)
        }

    internal fun infer(
        manifestDirectory: List<String>,
        targetDirectory: List<String>,
        mappings: List<AutoloadMapping>,
    ): String? = inferPlacement(manifestDirectory, targetDirectory, mappings)?.namespace

    internal fun inferPlacement(
        manifestDirectory: List<String>,
        targetDirectory: List<String>,
        mappings: List<AutoloadMapping>,
    ): SourcePlacement? {
        if (!targetDirectory.startsWith(manifestDirectory)) return null

        val matches = mappings.mapNotNull { mapping ->
            val root = manifestDirectory + mapping.directory
            if (!targetDirectory.startsWith(root)) return@mapNotNull null
            val suffix = targetDirectory.drop(root.size)
            if (suffix.any { !DORIA_IDENTIFIER.matches(it) }) return@mapNotNull null
            Match(mapping.namespace + suffix, mapping.directory.size, mapping.development)
        }
        if (matches.isEmpty()) return null

        val longestPath = matches.maxOf(Match::pathLength)
        val authoritative = matches.filter { it.pathLength == longestPath }
        val namespace = authoritative.map { it.namespace.joinToString("\\") }.distinct().singleOrNull()
            ?: return null
        // Development scope only when every authoritative mapping is `autoload-dev`.
        return SourcePlacement(namespace, development = authoritative.all(Match::development))
    }

    private fun namespaceSegments(prefix: String): List<String>? {
        val segments = prefix.trimEnd('\\').let { normalized ->
            if (normalized.isEmpty()) emptyList() else normalized.split('\\')
        }
        return segments.takeIf { it.all(DORIA_IDENTIFIER::matches) }
    }

    private fun mappingPathSegments(path: String): List<String>? {
        if (path.startsWith('/') || WINDOWS_ABSOLUTE_PATH.containsMatchIn(path)) return null
        val segments = path.split('/', '\\')
            .filter { it.isNotEmpty() && it != "." }
        return segments.takeIf { ".." !in it }
    }

    private fun pathSegments(file: VirtualFile): List<String> =
        file.path.split('/').filter(String::isNotEmpty)

    private fun List<String>.startsWith(prefix: List<String>): Boolean =
        size >= prefix.size && take(prefix.size) == prefix

    internal data class AutoloadMapping(
        val namespace: List<String>,
        val directory: List<String>,
        /** Whether `autoload-dev` declares the mapping. */
        val development: Boolean,
    )

    /** Where Baton places sources of a directory. */
    internal data class SourcePlacement(val namespace: String, val development: Boolean)

    private data class Match(val namespace: List<String>, val pathLength: Int, val development: Boolean)

    private const val MANIFEST_NAME = "Baton.toml"
    private const val NAMESPACES = "namespaces"
    private val TABLE_NAMES = listOf(
        "autoload" to false,
        "autoload-dev" to true,
    )
    private val DORIA_IDENTIFIER = Regex("[A-Za-z_][A-Za-z0-9_]*")
    private val WINDOWS_ABSOLUTE_PATH = Regex("^[A-Za-z]:[\\\\/]")
}
