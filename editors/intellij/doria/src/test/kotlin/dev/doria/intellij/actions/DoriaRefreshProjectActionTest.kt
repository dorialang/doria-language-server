package dev.doria.intellij.actions

import junit.framework.TestCase

class DoriaRefreshProjectActionTest : TestCase() {
    fun testOptionalLspDescriptorRegistersCanonicalProjectRefreshAction() {
        val descriptor = requireNotNull(
            javaClass.getResource("/META-INF/doria-lsp.xml"),
        ).readText()

        assertTrue(descriptor.contains("dev.doria.intellij.actions.DoriaRefreshProjectAction"))
        assertEquals("doria.refreshProject", DoriaRefreshProjectAction.COMMAND)
    }
}
