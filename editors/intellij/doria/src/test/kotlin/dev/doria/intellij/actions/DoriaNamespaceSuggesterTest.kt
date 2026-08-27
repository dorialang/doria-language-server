package dev.doria.intellij.actions

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class DoriaNamespaceSuggesterTest {
    @Test
    fun readsOnlyTheNamespaceDeclarationThroughTheDoriaLexer() {
        assertEquals(
            "Acme\\Blog\\Model",
            DoriaNamespaceSuggester.declaredNamespace(
                "// namespace Wrong;\nnamespace Acme\\Blog\\Model;\nuse Vendor\\Package\\Type;",
            ),
        )
        assertNull(DoriaNamespaceSuggester.declaredNamespace("use Acme\\Blog\\Model;"))
    }

    @Test
    fun keepsTheNamespaceUsedByTheTargetDirectory() {
        assertEquals(
            "Acme\\Blog\\Model",
            DoriaNamespaceSuggester.inferNamespace(
                listOf("workspace", "src", "Model"),
                listOf("workspace", "src", "Model"),
                "Acme\\Blog\\Model",
            ),
        )
    }

    @Test
    fun infersSiblingAndNestedNamespacesFromObservedLayout() {
        assertEquals(
            "Acme\\Blog\\Http\\Controller",
            DoriaNamespaceSuggester.inferNamespace(
                listOf("workspace", "src", "Http", "Controller"),
                listOf("workspace", "src", "Model"),
                "Acme\\Blog\\Model",
            ),
        )
    }

    @Test
    fun representsTheRootNamespaceWithAnEmptySuggestion() {
        assertEquals(
            "",
            DoriaNamespaceSuggester.inferNamespace(
                listOf("workspace", "src"),
                listOf("workspace", "src", "Model"),
                "Model",
            ),
        )
    }

    @Test
    fun rejectsAFileWhoseNamespaceDoesNotDescribeItsDirectoryLayout() {
        assertNull(
            DoriaNamespaceSuggester.inferNamespace(
                listOf("workspace", "tests"),
                listOf("workspace", "src", "Model"),
                "Acme\\Blog\\Model",
            ),
        )
    }

    @Test
    fun rejectsDirectorySegmentsThatCannotBeNamespaceSegments() {
        assertNull(
            DoriaNamespaceSuggester.inferNamespace(
                listOf("workspace", "src", "http-api"),
                listOf("workspace", "src"),
                "Acme\\Blog",
            ),
        )
    }

    @Test
    fun usesAConfiguredNamespaceAsTheAuthoritativeSuggestion() {
        assertEquals(
            "Acme\\Configured",
            DoriaNamespaceSuggester.chooseUnambiguous(
                "Acme\\Configured",
                listOf("Observed\\One", "Observed\\Two"),
            ),
        )
    }

    @Test
    fun declinesAmbiguousObservedNamespaces() {
        assertNull(
            DoriaNamespaceSuggester.chooseUnambiguous(
                null,
                listOf("Observed\\One", "Observed\\Two"),
            ),
        )
    }
}
