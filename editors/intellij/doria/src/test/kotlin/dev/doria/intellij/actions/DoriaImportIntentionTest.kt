package dev.doria.intellij.actions

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class DoriaImportIntentionTest {
    @Test
    fun `import intention has stable user-facing identity`() {
        val intention = DoriaImportIntention()

        assertEquals("Use import", intention.text)
        assertEquals("Doria imports", intention.familyName)
        assertFalse(intention.startInWriteAction())
    }
}
