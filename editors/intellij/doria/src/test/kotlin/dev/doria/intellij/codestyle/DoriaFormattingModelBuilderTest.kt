package dev.doria.intellij.codestyle

import com.intellij.openapi.command.WriteCommandAction
import com.intellij.application.options.CodeStyle
import com.intellij.psi.codeStyle.CodeStyleManager
import com.intellij.psi.codeStyle.CommonCodeStyleSettings
import com.intellij.testFramework.fixtures.BasePlatformTestCase
import dev.doria.intellij.DoriaFileType
import dev.doria.intellij.DoriaLanguage

class DoriaFormattingModelBuilderTest : BasePlatformTestCase() {
    fun testDeclarationBraceSettingsDriveReformatting() {
        val nextLineSource = """
            class Greeter
            {
                function greet(): void
                {
                }
            }
        """.trimIndent()
        val common = CodeStyle.getSettings(project).getCommonSettings(DoriaLanguage)
        common.CLASS_BRACE_STYLE = CommonCodeStyleSettings.END_OF_LINE
        common.METHOD_BRACE_STYLE = CommonCodeStyleSettings.END_OF_LINE

        assertFormatted(
            nextLineSource,
            """
            class Greeter {
                function greet(): void {
                }
            }
            """.trimIndent(),
        )

        common.CLASS_BRACE_STYLE = CommonCodeStyleSettings.NEXT_LINE
        common.METHOD_BRACE_STYLE = CommonCodeStyleSettings.NEXT_LINE
        assertFormatted(myFixture.editor.document.text, nextLineSource)
    }

    fun testControlFlowContinuationSettingsDriveReformatting() {
        val common = CodeStyle.getSettings(project).getCommonSettings(DoriaLanguage)
        common.ELSE_ON_NEW_LINE = true
        common.CATCH_ON_NEW_LINE = true
        common.FINALLY_ON_NEW_LINE = true

        assertFormatted(
            """
            if (true) {
            } else {
            }
            try {
            } catch (Error ${'$'}error) {
            } finally {
            }
            """.trimIndent(),
            """
            if (true) {
            }
            else {
            }
            try {
            }
            catch (Error ${'$'}error) {
            }
            finally {
            }
            """.trimIndent(),
        )
    }

    fun testDoWhileFinalizerPlacementFollowsFinallySetting() {
        val common = CodeStyle.getSettings(project).getCommonSettings(DoriaLanguage)
        common.FINALLY_ON_NEW_LINE = true

        assertFormatted(
            """
            do {
            } while (false) finally {
            }
            """.trimIndent(),
            """
            do {
            } while (false)
            finally {
            }
            """.trimIndent(),
        )

        common.FINALLY_ON_NEW_LINE = false
        assertFormatted(
            myFixture.editor.document.text,
            """
            do {
            } while (false) finally {
            }
            """.trimIndent(),
        )
    }

    fun testIndentOptionsDriveReformatting() {
        configureIndentOptions(indentSize = 2, continuationIndentSize = 3, useTabs = false)

        assertFormatted(
            """
            function main(): void
            {
            if (
            true
            ) {
            echo "yes";
            }
            }
            """.trimIndent(),
            """
            function main(): void
            {
              if (
                 true
              ) {
                echo "yes";
              }
            }
            """.trimIndent(),
        )
    }

    fun testSpacingAndBraceSettingsDriveReformatting() {
        configureIndentOptions(indentSize = 4, continuationIndentSize = 4, useTabs = false)
        val common = CodeStyle.getSettings(project).getCommonSettings(DoriaLanguage)
        common.SPACE_AROUND_ASSIGNMENT_OPERATORS = false
        common.SPACE_BEFORE_METHOD_CALL_PARENTHESES = true
        common.SPACE_BEFORE_METHOD_PARENTHESES = false
        common.SPACE_WITHIN_METHOD_CALL_PARENTHESES = true
        common.SPACE_WITHIN_METHOD_PARENTHESES = false
        common.METHOD_BRACE_STYLE = CommonCodeStyleSettings.END_OF_LINE

        assertFormatted(
            """
            function greet(string ${'$'}name):string
            {
            let ${'$'}message = greet(${'$'}name);
            }
            """.trimIndent(),
            """
            function greet(string ${'$'}name): string {
                let ${'$'}message=greet ( ${'$'}name );
            }
            """.trimIndent(),
        )
    }

    fun testTabsAndLanguageBoundariesRemainSemantic() {
        configureIndentOptions(indentSize = 4, continuationIndentSize = 4, useTabs = true)

        assertFormatted(
            """
            namespace App\Services;

            #[Route(path: "/users")]
            class Repository
            {
            function first(Dictionary<string, List<int>> ${'$'}items): ?string
            {
            let ${'$'}message = "Hello, {${'$'}this->name}!";
            return ${'$'}items[0];
            }
            }
            """.trimIndent(),
            """
            |namespace App\Services;
            |
            |#[Route(path: "/users")]
            |class Repository
            |{
            |\tfunction first(Dictionary<string, List<int>> ${'$'}items): ?string
            |\t{
            |\t\tlet ${'$'}message = "Hello, {${'$'}this->name}!";
            |\t\treturn ${'$'}items[0];
            |\t}
            |}
            """.trimMargin().replace("\\t", "\t"),
        )
    }

    fun testWordOperatorsKeepRequiredLexicalSpacing() {
        configureIndentOptions(indentSize = 4, continuationIndentSize = 4, useTabs = false)
        val common = CodeStyle.getSettings(project).getCommonSettings(DoriaLanguage)
        common.SPACE_AROUND_LOGICAL_OPERATORS = false
        common.SPACE_AROUND_RELATIONAL_OPERATORS = false

        assertFormatted(
            """
            function accepts(mixed ${'$'}value): bool
            {
            return ${'$'}value is string and true xor false;
            }
            """.trimIndent(),
            """
            function accepts(mixed ${'$'}value): bool
            {
                return ${'$'}value is string and true xor false;
            }
            """.trimIndent(),
        )
    }

    private fun configureIndentOptions(
        indentSize: Int,
        continuationIndentSize: Int,
        useTabs: Boolean,
    ) {
        val options = CodeStyle.getSettings(project).getLanguageIndentOptions(DoriaLanguage)
        options.INDENT_SIZE = indentSize
        options.CONTINUATION_INDENT_SIZE = continuationIndentSize
        options.TAB_SIZE = indentSize
        options.USE_TAB_CHARACTER = useTabs
    }

    private fun assertFormatted(before: String, expected: String) {
        val file = myFixture.configureByText(DoriaFileType.INSTANCE, before)
        WriteCommandAction.runWriteCommandAction(project) {
            CodeStyleManager.getInstance(project).reformat(file)
        }
        assertEquals(expected, myFixture.editor.document.text)
    }
}
