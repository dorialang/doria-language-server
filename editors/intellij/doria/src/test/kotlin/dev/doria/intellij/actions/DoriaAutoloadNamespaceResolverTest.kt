package dev.doria.intellij.actions

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class DoriaAutoloadNamespaceResolverTest {
    @Test
    fun infersTheNamespaceFromTheMatchingAutoloadRoot() {
        val mappings = mappings(
            """
            [autoload.namespaces]
            "Acme\\ImageTools\\" = "src/"
            """.trimIndent(),
        )

        assertEquals("Acme\\ImageTools", infer(listOf("project", "src"), mappings))
        assertEquals("Acme\\ImageTools\\Transform", infer(listOf("project", "src", "Transform"), mappings))
    }

    @Test
    fun readsDevelopmentAndAdvancedMappings() {
        val mappings = mappings(
            """
            [autoload-dev.namespaces]
            "Acme\\ImageTools\\Tests\\" = { path = "tests/", include = ["**/*.doria"] }
            """.trimIndent(),
        )

        assertEquals("Acme\\ImageTools\\Tests\\Unit", infer(listOf("project", "tests", "Unit"), mappings))
    }

    @Test
    fun givesTheLongestMatchingDirectoryRootAuthority() {
        val mappings = mappings(
            """
            [autoload.namespaces]
            "Acme\\" = "src/"
            "Acme\\Generated\\" = "src/Generated/"
            """.trimIndent(),
        )

        assertEquals("Acme\\Generated\\Api", infer(listOf("project", "src", "Generated", "Api"), mappings))
    }

    @Test
    fun declinesAmbiguousEqualRootsInsteadOfGuessing() {
        val mappings = mappings(
            """
            [autoload.namespaces]
            "Acme\\" = "src/"

            [autoload-dev.namespaces]
            "Tests\\" = "src/"
            """.trimIndent(),
        )

        assertNull(infer(listOf("project", "src", "Unit"), mappings))
    }

    @Test
    fun ignoresInvalidOrEscapingMappings() {
        assertEquals(emptyList<DoriaAutoloadNamespaceResolver.AutoloadMapping>(), mappings("[autoload.namespaces"))
        assertNull(
            infer(
                listOf("project", "src"),
                mappings(
                    """
                    [autoload.namespaces]
                    "Acme\\" = "../src/"
                    """.trimIndent(),
                ),
            ),
        )
    }

    @Test
    fun ignoresDirectoriesOutsideTheConfiguredRoots() {
        val mappings = mappings(
            """
            [autoload.namespaces]
            "Acme\\" = "src/"
            """.trimIndent(),
        )

        assertNull(infer(listOf("project", "examples"), mappings))
    }

    private fun mappings(source: String) = DoriaAutoloadNamespaceResolver.mappings(source)

    private fun infer(
        target: List<String>,
        mappings: List<DoriaAutoloadNamespaceResolver.AutoloadMapping>,
    ): String? = DoriaAutoloadNamespaceResolver.infer(listOf("project"), target, mappings)
}
