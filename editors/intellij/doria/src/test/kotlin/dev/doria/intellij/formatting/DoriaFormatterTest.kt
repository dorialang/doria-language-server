package dev.doria.intellij.formatting

import com.intellij.application.options.CodeStyle
import com.intellij.openapi.command.WriteCommandAction
import com.intellij.psi.codeStyle.CodeStyleManager
import com.intellij.psi.codeStyle.CommonCodeStyleSettings
import com.intellij.testFramework.fixtures.BasePlatformTestCase
import dev.doria.intellij.DoriaFileType

class DoriaFormatterTest : BasePlatformTestCase() {
    fun testBracePlacementChangesFormattedOutput() {
        val nextLineSource = """
            class Greeter
            {
                function greet(): void
                {
                }
            }
        """.trimIndent() + "\n"

        myFixture.configureByText(DoriaFileType.INSTANCE, nextLineSource)
        val commonSettings = CodeStyle.getLanguageSettings(myFixture.file)

        commonSettings.BRACE_STYLE = CommonCodeStyleSettings.END_OF_LINE
        reformat()
        assertEquals(
            """
                class Greeter {
                    function greet(): void {
                    }
                }
            """.trimIndent() + "\n",
            myFixture.file.text,
        )

        commonSettings.BRACE_STYLE = CommonCodeStyleSettings.NEXT_LINE
        reformat()
        assertEquals(nextLineSource, myFixture.file.text)
    }

    fun testElsePlacementChangesFormattedOutput() {
        myFixture.configureByText(
            DoriaFileType.INSTANCE,
            "if (true) {\n} else {\n}\n",
        )
        val commonSettings = CodeStyle.getLanguageSettings(myFixture.file)
        commonSettings.BRACE_STYLE = CommonCodeStyleSettings.END_OF_LINE
        commonSettings.ELSE_ON_NEW_LINE = true

        reformat()

        assertEquals("if (true) {\n}\nelse {\n}\n", myFixture.file.text)
    }

    fun testCatchAndFinallyPlacementChangesFormattedOutput() {
        myFixture.configureByText(
            DoriaFileType.INSTANCE,
            "try {\n} catch (Error ${'$'}error) {\n} finally {\n}\n",
        )
        val commonSettings = CodeStyle.getLanguageSettings(myFixture.file)
        commonSettings.BRACE_STYLE = CommonCodeStyleSettings.END_OF_LINE
        commonSettings.CATCH_ON_NEW_LINE = true
        commonSettings.FINALLY_ON_NEW_LINE = true

        reformat()

        assertEquals(
            "try {\n}\ncatch (Error ${'$'}error) {\n}\nfinally {\n}\n",
            myFixture.file.text,
        )
    }

    fun testConfiguredIndentSizeIsApplied() {
        myFixture.configureByText(
            DoriaFileType.INSTANCE,
            "class Greeter {\n    function greet(): void {\n    }\n}\n",
        )
        val commonSettings = CodeStyle.getLanguageSettings(myFixture.file)
        commonSettings.indentOptions?.INDENT_SIZE = 2

        reformat()

        assertEquals(
            "class Greeter {\n  function greet(): void {\n  }\n}\n",
            myFixture.file.text,
        )
    }

    private fun reformat() {
        WriteCommandAction.runWriteCommandAction(project) {
            CodeStyleManager.getInstance(project).reformatText(
                myFixture.file,
                listOf(myFixture.file.textRange),
            )
        }
    }
}
