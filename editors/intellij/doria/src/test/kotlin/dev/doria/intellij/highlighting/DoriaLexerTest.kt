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

    fun testStage22TypeTestAndRejectedPhpOperator() {
        val tokens = lex("if (\$payload is string) { echo \$payload instanceof User; }")

        assertEquals(DoriaTokenTypes.TYPE_TEST_OPERATOR, tokens.first { it.text == "is" }.type)
        assertEquals(DoriaTokenTypes.INVALID, tokens.first { it.text == "instanceof" }.type)
    }

    fun testNeverIsNotAdvertisedAsAPrimitiveType() {
        val tokens = lex("never \$result = fail();")

        assertEquals(DoriaTokenTypes.IDENTIFIER, tokens.first { it.text == "never" }.type)
    }

    fun testSequenceFillSeparatorRemainsValidPunctuation() {
        val tokens = lex("let \$flags = [true; \$count];")

        assertEquals(DoriaTokenTypes.BRACKET, tokens.first { it.text == "[" }.type)
        assertEquals(DoriaTokenTypes.PUNCTUATION, tokens.first { it.text == ";" }.type)
        assertEquals(DoriaTokenTypes.BRACKET, tokens.first { it.text == "]" }.type)
        assertFalse(tokens.any { it.type == DoriaTokenTypes.INVALID })
    }

    fun testGenericFunctionNameRemainsADeclaration() {
        val tokens = lex("function first<T>(List<T> \$items): ?T { return \$items->first; }")

        assertEquals(
            DoriaTokenTypes.FUNCTION_DECLARATION,
            tokens.first { it.text == "first" }.type,
        )
        assertFalse(tokens.any { it.type == DoriaTokenTypes.INVALID })
    }

    fun testGenericClassDeclarationsAndInstantiationsRemainValidTokens() {
        val tokens = lex(
            "class Box<T implements Displayable> {} " +
                "function use(Box<int> \$box): void {}",
        )

        assertEquals(DoriaTokenTypes.KEYWORD, tokens.first { it.text == "class" }.type)
        assertEquals(DoriaTokenTypes.TYPE_NAME, tokens.first { it.text == "Box" }.type)
        assertFalse(tokens.any { it.type == DoriaTokenTypes.INVALID })
    }

    fun testSharedOwnershipTypesAndReferencedValueUseTypeAndPropertyTokens() {
        val tokens = lex(
            "SharedReference<User> \$user; " +
                "WritableSharedReferenceAccess<User> \$access; " +
                "echo \$user->referencedValue->name;",
        )

        assertEquals(
            DoriaTokenTypes.COLLECTION_TYPE,
            tokens.first { it.text == "SharedReference" }.type,
        )
        assertEquals(
            DoriaTokenTypes.COLLECTION_TYPE,
            tokens.first { it.text == "WritableSharedReferenceAccess" }.type,
        )
        assertEquals(
            DoriaTokenTypes.PROPERTY,
            tokens.first { it.text == "referencedValue" }.type,
        )
    }

    fun testCompleteCollectionFamilyUsesCollectionTypeHighlighting() {
        val tokens = lex(
            "SortedDictionary<int, string> SortedSet<int> PriorityQueue<int> Deque<string>",
        )

        for (name in listOf("SortedDictionary", "SortedSet", "PriorityQueue", "Deque")) {
            assertEquals(DoriaTokenTypes.COLLECTION_TYPE, tokens.first { it.text == name }.type)
        }
        assertFalse(tokens.any { it.type == DoriaTokenTypes.INVALID })
    }

    fun testEnumDeclarationsCasesAndMatchKeywordsUseAcceptedTokenKinds() {
        val tokens = lex(
            "enum Status { case Draft; case Ready(int \$code); } " +
                "Status \$status = Status::Draft; " +
                "Status \$ready = Status::Ready(code: 42); " +
                "let \$label = match (\$status) { " +
                "Status::Ready(\$value) => \$value, Status::Draft => 1, default => 0, };",
        )

        assertEquals(DoriaTokenTypes.ENUM_DECLARATION, tokens.first { it.text == "Status" }.type)
        for (token in tokens.filter { it.text == "Draft" }) {
            assertEquals(DoriaTokenTypes.ENUM_CASE, token.type)
        }
        for (token in tokens.filter { it.text == "Ready" }) {
            assertEquals(DoriaTokenTypes.ENUM_CASE, token.type)
        }
        for (keyword in listOf("enum", "case", "match", "default")) {
            assertEquals(DoriaTokenTypes.KEYWORD, tokens.first { it.text == keyword }.type)
        }
        for (binding in tokens.filter { it.text == "\$value" }) {
            assertEquals(DoriaTokenTypes.VARIABLE, binding.type)
        }
        assertFalse(tokens.any { it.type == DoriaTokenTypes.INVALID })
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
