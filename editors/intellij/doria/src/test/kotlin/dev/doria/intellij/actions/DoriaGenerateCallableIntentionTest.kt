package dev.doria.intellij.actions

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class DoriaGenerateCallableIntentionTest {
    @Test
    fun `method intention has stable user-facing identity`() {
        val intention = DoriaGenerateMethodIntention()

        assertEquals("Generate missing method", intention.text)
        assertEquals("Doria code generation", intention.familyName)
        assertFalse(intention.startInWriteAction())
    }

    @Test
    fun `function intention has stable user-facing identity`() {
        val intention = DoriaGenerateFunctionIntention()

        assertEquals("Generate missing function", intention.text)
        assertEquals("Doria code generation", intention.familyName)
        assertFalse(intention.startInWriteAction())
    }
}
