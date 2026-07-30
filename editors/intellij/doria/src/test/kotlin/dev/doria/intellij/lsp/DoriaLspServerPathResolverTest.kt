package dev.doria.intellij.lsp

import com.intellij.openapi.util.SystemInfo
import junit.framework.TestCase

class DoriaLspServerPathResolverTest : TestCase() {
    fun testCargoInstallRootWinsOverCargoHome() {
        val candidates = DoriaLspServerPathResolver.cargoBinCandidates(
            mapOf(
                "CARGO_INSTALL_ROOT" to "/toolchain",
                "CARGO_HOME" to "/cargo",
            ),
            "/home/test",
        )

        assertEquals(
            "/toolchain/bin/${if (SystemInfo.isWindows) "doria-lsp.exe" else "doria-lsp"}",
            candidates.single().toString().replace('\\', '/'),
        )
    }

    fun testCargoHomeIsPlatformNeutralInstalledFallback() {
        val candidates = DoriaLspServerPathResolver.cargoBinCandidates(
            mapOf("CARGO_HOME" to "/cargo"),
            "/home/test",
        )

        assertEquals(
            "/cargo/bin/${if (SystemInfo.isWindows) "doria-lsp.exe" else "doria-lsp"}",
            candidates.single().toString().replace('\\', '/'),
        )
    }
}
