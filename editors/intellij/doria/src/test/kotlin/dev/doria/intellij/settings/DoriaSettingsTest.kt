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
}
