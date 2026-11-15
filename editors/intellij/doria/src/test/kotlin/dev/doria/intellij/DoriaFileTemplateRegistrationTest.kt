package dev.doria.intellij

import junit.framework.TestCase
import java.nio.file.FileSystems
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths

/**
 * The platform asserts "Unknown bundled file template" when an action asks for
 * an internal template that plugin.xml does not register, so every template
 * under fileTemplates/internal needs an <internalFileTemplate> entry, and
 * every entry needs its template.
 */
class DoriaFileTemplateRegistrationTest : TestCase() {
    fun testEveryInternalTemplateIsRegistered() {
        val bundled = bundledTemplateNames()
        val descriptor = requireNotNull(javaClass.getResource("/META-INF/plugin.xml")).readText()
        val registered = Regex("""<internalFileTemplate\s+name="([^"]+)"""")
            .findAll(descriptor)
            .map { it.groupValues[1] }
            .toSortedSet()

        assertFalse("no bundled internal templates were found", bundled.isEmpty())
        assertEquals(bundled, registered)
    }

    /** Lists the templates whether the resources are a directory or a jar. */
    private fun bundledTemplateNames(): Set<String> {
        val uri = requireNotNull(javaClass.getResource("/fileTemplates/internal")).toURI()
        fun names(directory: Path): Set<String> = Files.list(directory).use { entries ->
            entries
                .map { it.fileName.toString() }
                .filter { it.endsWith(".doria.ft") }
                .map { it.removeSuffix(".doria.ft") }
                .toList()
                .toSortedSet()
        }
        if (uri.scheme != "jar") return names(Paths.get(uri))
        return FileSystems.newFileSystem(uri, emptyMap<String, Any>()).use { jar ->
            names(jar.getPath("/fileTemplates/internal"))
        }
    }
}
