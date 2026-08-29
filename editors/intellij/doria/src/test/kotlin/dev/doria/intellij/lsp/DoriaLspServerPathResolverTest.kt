package dev.doria.intellij.lsp

import com.intellij.openapi.util.SystemInfo
import junit.framework.TestCase
import kotlin.io.path.createTempDirectory
import java.nio.file.Paths

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

    fun testEveryReleasePlatformHasOneStableBundleKey() {
        val cases = mapOf(
            Pair("Linux", "amd64") to "linux-x86_64",
            Pair("Linux", "aarch64") to "linux-aarch64",
            Pair("Mac OS X", "x86_64") to "macos-x86_64",
            Pair("Darwin", "arm64") to "macos-aarch64",
            Pair("Windows 11", "AMD64") to "windows-x86_64",
            Pair("Windows 11", "aarch64") to "windows-aarch64",
        )

        for ((host, expected) in cases) {
            assertEquals(expected, DoriaLspServerPathResolver.platformKey(host.first, host.second))
        }
    }

    fun testUnsupportedPlatformsDoNotSelectAnotherNativeBinary() {
        assertNull(DoriaLspServerPathResolver.platformKey("FreeBSD", "amd64"))
        assertNull(DoriaLspServerPathResolver.platformKey("Linux", "riscv64"))
    }

    fun testBundledCandidateUsesThePluginPlatformDirectory() {
        val plugin = createTempDirectory("doria-plugin-")
        try {
            val binary = plugin.resolve("bin/macos-aarch64/doria-lsp")
            binary.parent.toFile().mkdirs()
            binary.toFile().writeText("server")

            assertEquals(
                binary,
                DoriaLspServerPathResolver.bundledCandidate(plugin, "Mac OS X", "arm64"),
            )
            assertNull(
                DoriaLspServerPathResolver.bundledCandidate(
                    Paths.get(plugin.toString()),
                    "Windows 11",
                    "amd64",
                ),
            )
        } finally {
            plugin.toFile().deleteRecursively()
        }
    }

    fun testConfiguredToolPathsExpandProjectAndHomePortably() {
        assertEquals(
            "/workspace/tools/baton",
            DoriaLspServerPathResolver.expandConfiguredPath(
                "\$PROJECT_DIR$/tools/baton",
                "/workspace",
                "/home/test",
            ),
        )
        assertEquals(
            "/home/test/bin/baton",
            DoriaLspServerPathResolver.expandConfiguredPath(
                "~/bin/baton",
                null,
                "/home/test",
            ),
        )
    }
}
