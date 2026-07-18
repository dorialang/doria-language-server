package dev.doria.intellij.highlighting

import com.intellij.psi.tree.IElementType
import junit.framework.TestCase

class DoriaLexerTest : TestCase() {
    fun testNestedAttributeBracketsDoNotCloseTheAttributeEarly() {
        val tokens = lex("#[Module(imports: [Factory::make(values: [])])] class App {}")

        assertEquals(DoriaTokenTypes.ATTRIBUTE_DELIMITER, tokens.first().type)
        assertEquals("#[", tokens.first().text)
        assertEquals(DoriaTokenTypes.ATTRIBUTE_DELIMITER, tokens.last { it.text == "]" }.type)
        assertEquals(4, tokens.count { it.type == DoriaTokenTypes.BRACKET })
        assertEquals(DoriaTokenTypes.ATTRIBUTE_ARGUMENT, tokens.first { it.text == "imports" }.type)
        assertEquals(DoriaTokenTypes.ATTRIBUTE_ARGUMENT, tokens.first { it.text == "values" }.type)
        assertEquals(DoriaTokenTypes.KEYWORD, tokens.first { it.text == "class" }.type)
    }

    private fun lex(source: String): List<Token> {
        val lexer = DoriaLexer()
        lexer.start(source)
        return buildList {
            while (lexer.tokenType != null) {
                add(Token(lexer.tokenType!!, source.substring(lexer.tokenStart, lexer.tokenEnd)))
                lexer.advance()
            }
        }
    }

    private data class Token(val type: IElementType, val text: String)
}
