package dev.doria.intellij.highlighting

import com.intellij.psi.tree.IElementType
import com.intellij.testFramework.fixtures.BasePlatformTestCase

class DoriaLexerTest : BasePlatformTestCase() {
    fun testStage22TypeTestAndRejectedPhpOperator() {
        val tokens = lex("if (\$payload is string) { echo \$payload instanceof User; }")

        assertEquals(DoriaTokenTypes.TYPE_TEST_OPERATOR, tokens.getValue("is"))
        assertEquals(DoriaTokenTypes.INVALID, tokens.getValue("instanceof"))
    }

    fun testNeverIsNotAdvertisedAsAPrimitiveType() {
        val tokens = lex("never \$result = fail();")

        assertEquals(DoriaTokenTypes.IDENTIFIER, tokens.getValue("never"))
    }

    private fun lex(source: String): Map<String, IElementType> {
        val lexer = DoriaLexer()
        val tokens = mutableMapOf<String, IElementType>()
        lexer.start(source)
        while (lexer.tokenType != null) {
            tokens[source.substring(lexer.tokenStart, lexer.tokenEnd)] = lexer.tokenType!!
            lexer.advance()
        }
        return tokens
    }
}
