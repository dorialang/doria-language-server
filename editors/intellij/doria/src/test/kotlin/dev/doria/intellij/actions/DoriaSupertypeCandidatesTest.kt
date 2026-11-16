package dev.doria.intellij.actions

import com.google.gson.JsonParser
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class DoriaSupertypeCandidatesTest {
    @Test
    fun readsTheLanguageServerAnswer() {
        val candidates = DoriaSupertypeCandidates.parse(
            JsonParser.parseString(
                """
                {
                  "package": "acme/app",
                  "classes": [
                    { "label": "Lib\\Box", "qualifiedName": "Lib\\Box", "typeParameters": ["T", "U"] }
                  ],
                  "interfaces": [
                    { "label": "Renderable", "qualifiedName": "App\\Renderable", "typeParameters": [] }
                  ],
                  "keywords": ["class", "match"]
                }
                """.trimIndent(),
            ).asJsonObject,
        )

        assertEquals("acme/app", candidates.packageName)
        assertEquals("Lib\\Box<T, U>", candidates.classes.single().headerText)
        assertEquals("Renderable", candidates.interfaces.single().headerText)
        assertEquals("App\\Renderable", candidates.interfaces.single().qualifiedName)
        assertEquals(setOf("class", "match"), candidates.keywords)
    }

    @Test
    fun readsADirectoryOutsideEveryPackage() {
        val candidates = DoriaSupertypeCandidates.parse(
            JsonParser.parseString(
                """{ "package": null, "classes": [], "interfaces": [], "keywords": [] }""",
            ).asJsonObject,
        )

        assertNull(candidates.packageName)
        assertEquals(emptyList<DoriaSupertype>(), candidates.classes)
    }
}
