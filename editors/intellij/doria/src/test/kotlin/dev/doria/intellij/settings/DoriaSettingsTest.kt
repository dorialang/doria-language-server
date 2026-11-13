package dev.doria.intellij.settings

import junit.framework.TestCase

class DoriaSettingsTest : TestCase() {
    fun testLanguageServerAndBatonOverridesAreIndependent() {
        val state = DoriaSettings.State(
            languageServerPath = "/toolchain/doria-lsp",
            batonPath = "/toolchain/baton",
        )

        assertEquals("/toolchain/doria-lsp", state.languageServerPath)
        assertEquals("/toolchain/baton", state.batonPath)
    }

    fun testChangingEitherPathRestartsTheLanguageServer() {
        val before = DoriaSettings.State(languageServerPath = "", batonPath = "")

        assertFalse(DoriaConfigurable.requiresServerRestart(before, before.copy()))
        assertTrue(
            DoriaConfigurable.requiresServerRestart(before, before.copy(batonPath = "C:\\Doria\\bin\\baton.cmd")),
        )
        assertTrue(
            DoriaConfigurable.requiresServerRestart(before, before.copy(languageServerPath = "/toolchain/doria-lsp")),
        )
    }
}
