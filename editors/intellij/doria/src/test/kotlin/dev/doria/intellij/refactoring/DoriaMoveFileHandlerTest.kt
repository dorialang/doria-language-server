package dev.doria.intellij.refactoring

import org.junit.Assert.assertEquals
import org.junit.Test

class DoriaMoveFileHandlerTest {
    @Test
    fun replacesAnExistingFileScopeNamespace() {
        assertEquals(
            "namespace Acme\\Blog\\Http;\n\nclass Post {}\n",
            DoriaMoveFileHandler.updateNamespace(
                "namespace Acme\\Blog\\Model;\n\nclass Post {}\n",
                "Acme\\Blog\\Http",
            ),
        )
    }

    @Test
    fun addsANamespaceWhenMovingRootSourceIntoANamespacedDirectory() {
        assertEquals(
            "namespace Acme\\Blog;\n\nclass Post {}\n",
            DoriaMoveFileHandler.updateNamespace("class Post {}\n", "Acme\\Blog"),
        )
    }

    @Test
    fun removesTheDeclarationWhenMovingIntoTheRootNamespace() {
        assertEquals(
            "class Post {}\n",
            DoriaMoveFileHandler.updateNamespace(
                "namespace Acme\\Blog;\n\nclass Post {}\n",
                "",
            ),
        )
    }

    @Test
    fun ignoresNamespaceWordsInsideComments() {
        assertEquals(
            "namespace Acme\\Blog;\n\n// namespace Wrong;\nclass Post {}\n",
            DoriaMoveFileHandler.updateNamespace(
                "// namespace Wrong;\nclass Post {}\n",
                "Acme\\Blog",
            ),
        )
    }
}
