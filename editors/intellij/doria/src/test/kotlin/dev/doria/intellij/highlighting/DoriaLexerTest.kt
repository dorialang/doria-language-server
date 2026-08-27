package dev.doria.intellij.highlighting

import com.intellij.psi.tree.IElementType
import junit.framework.TestCase

class DoriaLexerTest : TestCase() {
    fun testTopLevelInternalDeclarationsKeepModifierAndDeclarationHighlighting() {
        val tokens = lex(
            "internal class Helper {} " +
                "internal enum State { case Ready; } " +
                "internal interface Contract {} " +
                "internal trait Support {} " +
                "internal function helper(): void {} " +
                "internal const int LIMIT = 10;",
        )

        for (token in tokens.filter { it.text == "internal" }) {
            assertEquals(DoriaTokenTypes.MODIFIER, token.type)
        }
        for (name in listOf("Helper", "Contract", "Support")) {
            assertEquals(DoriaTokenTypes.TYPE_NAME, tokens.first { it.text == name }.type)
        }
        assertEquals(DoriaTokenTypes.ENUM_DECLARATION, tokens.first { it.text == "State" }.type)
        assertEquals(DoriaTokenTypes.FUNCTION_DECLARATION, tokens.first { it.text == "helper" }.type)
        assertEquals(DoriaTokenTypes.CLASS_CONSTANT, tokens.first { it.text == "LIMIT" }.type)
        assertFalse(tokens.any { it.type == DoriaTokenTypes.INVALID })
    }

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

    fun testStage32AttributePresentationPreservesHashCommentsAndMalformedBoundaries() {
        val tokens = lex(
            "#[Attribute] #[Acme\\Metadata\\Route(path: \"/\")] " +
                "# comment\n# [Test]\n#[] function invalid(): void {}",
        )

        for (name in listOf("Attribute", "Acme", "Metadata", "Route")) {
            assertEquals(DoriaTokenTypes.ATTRIBUTE_NAME, tokens.first { it.text == name }.type)
        }
        assertEquals(DoriaTokenTypes.ATTRIBUTE_ARGUMENT, tokens.first { it.text == "path" }.type)
        assertEquals(2, tokens.count { it.type == DoriaTokenTypes.COMMENT })
        assertFalse(
            tokens.any { it.text.isNotEmpty() && it.type == DoriaTokenTypes.ATTRIBUTE_NAME && it.text == "]" },
        )
        assertFalse(tokens.any { it.type.toString().contains("PARSER") })
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
                "let \$label = match (take \$ready) { " +
                "Status::Ready(\$value) if \$value > 0 => \$value, " +
                "Status::Ready(\$value) => 0, Status::Draft => 1, default => 0, };",
        )

        assertEquals(DoriaTokenTypes.ENUM_DECLARATION, tokens.first { it.text == "Status" }.type)
        for (token in tokens.filter { it.text == "Draft" }) {
            assertEquals(DoriaTokenTypes.ENUM_CASE, token.type)
        }
        for (token in tokens.filter { it.text == "Ready" }) {
            assertEquals(DoriaTokenTypes.ENUM_CASE, token.type)
        }
        for (keyword in listOf("enum", "case", "match", "if", "default")) {
            assertEquals(DoriaTokenTypes.KEYWORD, tokens.first { it.text == keyword }.type)
        }
        assertEquals(DoriaTokenTypes.MODIFIER, tokens.first { it.text == "take" }.type)
        for (binding in tokens.filter { it.text == "\$value" }) {
            assertEquals(DoriaTokenTypes.VARIABLE, binding.type)
        }
        assertFalse(tokens.any { it.type == DoriaTokenTypes.INVALID })
    }

    fun testMatchGuardsDoNotCreateWhenOrWhereAliases() {
        val tokens = lex("match (\$value) { Item \$item if true => \$item, }")
        assertEquals(DoriaTokenTypes.KEYWORD, tokens.first { it.text == "if" }.type)
        assertFalse(tokens.any { it.text == "where" })
        assertFalse(tokens.any { it.text == "when" })
    }

    fun testEveryAcceptedStage28aFinalizerAttachmentUsesKeywordHighlighting() {
        val tokens = lex(
            "if (true) {} finally {} " +
                "given { true; } if (true) {} finally {} " +
                "let \$a = when (true): int { return 1; } else { return 0; } finally {}; " +
                "let \$b = given { true; } when (true): int { return 1; } else { return 0; } finally {}; " +
                "while (true) {} finally {} " +
                "given { true; } while (true) {} finally {} " +
                "do {} while (false) finally {}",
        )

        for (keyword in listOf("given", "if", "when", "return", "else", "do", "while", "finally")) {
            for (token in tokens.filter { it.text == keyword }) {
                assertEquals(DoriaTokenTypes.KEYWORD, token.type)
            }
        }
        assertFalse(tokens.any { it.text == "elseif" })
        assertFalse(tokens.any { it.type == DoriaTokenTypes.INVALID })
    }

    fun testCheckedErrorSyntaxUsesExecutableKeywordAndTypeHighlighting() {
        val tokens = lex(
            "class Failure implements Error {} " +
                "function fail(): void throws Failure { throw new Failure(); } " +
                "function handle(): void { try { fail(); } catch (Failure \$error) {} finally {} }",
        )

        for (keyword in listOf("try", "catch", "throw", "throws", "finally")) {
            assertEquals(DoriaTokenTypes.KEYWORD, tokens.first { it.text == keyword }.type)
        }
        assertEquals(DoriaTokenTypes.TYPE_NAME, tokens.first { it.text == "Error" }.type)
        assertFalse(tokens.any { it.type == DoriaTokenTypes.INVALID })
    }

    fun testAcceptedClosureGrammarUsesAlignedKeywordModifierAndVariableTokens() {
        val tokens = lex(
            "let \$minimum = 70; " +
                "let writable \$count = 0; " +
                "let \$arrow = fn(int \$value) with (\$minimum) => \$value; " +
                "let \$block = function (int \$value): bool with (writable \$count, take \$message) { return true; }; " +
                "function transform(function(int): string \$mapper): function(string): int {}",
        )

        assertEquals(DoriaTokenTypes.KEYWORD, tokens.first { it.text == "fn" }.type)
        for (token in tokens.filter { it.text == "with" }) {
            assertEquals(DoriaTokenTypes.KEYWORD, token.type)
        }
        for (modifier in listOf("writable", "take")) {
            for (token in tokens.filter { it.text == modifier }) {
                assertEquals(DoriaTokenTypes.MODIFIER, token.type)
            }
        }
        for (capture in listOf("\$minimum", "\$count", "\$message")) {
            assertEquals(DoriaTokenTypes.VARIABLE, tokens.last { it.text == capture }.type)
        }
        for (token in tokens.filter { it.text == "function" }) {
            assertEquals(DoriaTokenTypes.KEYWORD, token.type)
        }
        assertEquals(
            DoriaTokenTypes.FUNCTION_DECLARATION,
            tokens.first { it.text == "transform" }.type,
        )
        assertFalse(tokens.any { it.type == DoriaTokenTypes.INVALID })
    }

    fun testStage30aFunctionTypesAndCallableCallsUseAlignedPresentationTokens() {
        val tokens = lex(
            "function writable(writable Counter): void \$writer; " +
                "function once(take Payload): string \$consumer; " +
                "function((function(): int throws ParseError), string): void \$nested; " +
                "\$consumer(1); transform(2); \$service->transform(3);",
        )

        assertEquals(DoriaTokenTypes.KEYWORD, tokens.first { it.text == "once" }.type)
        for (token in tokens.filter { it.text == "throws" }) {
            assertEquals(DoriaTokenTypes.KEYWORD, token.type)
        }
        for (token in tokens.filter { it.text == "writable" || it.text == "take" }) {
            assertEquals(DoriaTokenTypes.MODIFIER, token.type)
        }
        assertEquals(DoriaTokenTypes.VARIABLE, tokens.first { it.text == "\$consumer" }.type)
        assertEquals(DoriaTokenTypes.FUNCTION_CALL, tokens.first { it.text == "transform" }.type)
        assertEquals(DoriaTokenTypes.METHOD_CALL, tokens.last { it.text == "transform" }.type)
        assertFalse(tokens.any { it.type == DoriaTokenTypes.INVALID })
    }

    fun testOnceAndRejectedTakeInvocationRespectLexicalContext() {
        val tokens = lex(
            "function once(): Payload; function take(): Payload; " +
                "function(take Payload): void \$consumer; " +
                "onceOnly once_more once1 \$once; \"once\"; // once\n",
        )

        assertEquals(DoriaTokenTypes.KEYWORD, tokens.first { it.text == "once" }.type)
        assertEquals(DoriaTokenTypes.INVALID, tokens.first { it.text == "take" }.type)
        assertEquals(DoriaTokenTypes.MODIFIER, tokens.last { it.text == "take" }.type)
        for (identifier in listOf("onceOnly", "once_more", "once1")) {
            assertEquals(DoriaTokenTypes.IDENTIFIER, tokens.first { it.text == identifier }.type)
        }
        assertEquals(DoriaTokenTypes.VARIABLE, tokens.first { it.text == "\$once" }.type)
        assertEquals(DoriaTokenTypes.STRING, tokens.first { it.text == "\"once\"" }.type)
        assertEquals(DoriaTokenTypes.COMMENT, tokens.first { it.text.startsWith("//") }.type)

        for (source in listOf(
            "function /* comment */ take(): Payload;",
            "function // comment\n take(): Payload;",
            "function # comment\n take /* comment */ (): Payload;",
        )) {
            assertEquals(
                source,
                DoriaTokenTypes.INVALID,
                lex(source).first { it.text == "take" }.type,
            )
        }
        assertFalse(
            lex("// function\n take(): Payload;")
                .any { it.text == "take" && it.type == DoriaTokenTypes.INVALID },
        )
    }

    fun testClosureKeywordsRespectIdentifierStringAndCommentBoundaries() {
        val tokens = lex("fnord withdraw functionality; \"fn with\"; // fn with\n")

        for (identifier in listOf("fnord", "withdraw", "functionality")) {
            assertEquals(DoriaTokenTypes.IDENTIFIER, tokens.first { it.text == identifier }.type)
        }
        assertEquals(DoriaTokenTypes.STRING, tokens.first { it.text == "\"fn with\"" }.type)
        assertEquals(DoriaTokenTypes.COMMENT, tokens.first { it.text.startsWith("//") }.type)
        assertFalse(tokens.any { it.text == "fn" || it.text == "with" })
    }

    fun testCompilerOwnsMalformedCaptureDiagnosticsWithoutCommentFalsePositives() {
        for (source in listOf(
            "let \$f = fn(int \$value) with () => \$value;",
            "let \$f = fn(int \$value) with (&\$outside) => \$value;",
            "let \$f = fn(int \$value) with (writable &\$outside) => \$value;",
            "let \$f = fn(int \$value) with (readonly \$outside) => \$value;",
            "let \$f = fn(int \$value) with (\$outside /* readonly & */) => \$value;",
        )) {
            val tokens = lex(source)
            assertFalse(source, tokens.any { it.type == DoriaTokenTypes.INVALID })
        }

        val commentTokens = lex(
            "let \$f = fn(int \$value) with (\$outside /* readonly & */) => \$value;",
        )
        assertEquals(
            DoriaTokenTypes.COMMENT,
            commentTokens.first { it.text.startsWith("/*") }.type,
        )

        val legacyUse = lex("let \$f = fn(int \$value) use (\$outside) => \$value;")
        assertEquals(DoriaTokenTypes.INVALID, legacyUse.first { it.text == "use" }.type)
    }

    fun testNamespaceIndividualAliasAndGroupedImportsUsePresentationTokens() {
        val tokens = lex(
            "namespace Acme\\Editor;\n" +
                "use Acme\\Model\\User;\n" +
                "use Acme\\Http\\Client as HttpClient;\n" +
                "use Doria\\Std\\Math\\{\n" +
                "Vector2,\n" +
                "Vector3 as Position3,\n" +
                "};\n" +
                "include \"generated/routes.doria\";",
        )

        assertEquals(DoriaTokenTypes.KEYWORD, tokens.first { it.text == "namespace" }.type)
        for (segment in listOf("Acme", "Editor")) {
            assertEquals(
                DoriaTokenTypes.NAMESPACE_PATH,
                tokens.first { it.text == segment }.type,
            )
        }
        assertEquals(3, tokens.count { it.type == DoriaTokenTypes.IMPORT_USE_KEYWORD })
        for (alias in listOf("HttpClient", "Position3")) {
            assertEquals(DoriaTokenTypes.IMPORT_ALIAS, tokens.first { it.text == alias }.type)
        }
        for (entry in listOf("Vector2", "Vector3")) {
            assertEquals(DoriaTokenTypes.IMPORT_PATH, tokens.first { it.text == entry }.type)
        }
        assertEquals(DoriaTokenTypes.KEYWORD, tokens.first { it.text == "include" }.type)
        assertEquals(
            DoriaTokenTypes.STRING,
            tokens.first { it.text == "\"generated/routes.doria\"" }.type,
        )
    }

    fun testSingleSegmentNamespaceUsesNamespacePresentationTokens() {
        val tokens = lex("namespace First;")

        assertEquals(DoriaTokenTypes.KEYWORD, tokens.first { it.text == "namespace" }.type)
        assertEquals(DoriaTokenTypes.NAMESPACE_PATH, tokens.first { it.text == "First" }.type)
    }

    fun testRejectedNamespaceDirectiveShapesAreNotPresentedAsImports() {
        for (source in listOf(
            "use \\Acme\\Http\\Client;",
            "use Acme\\*;",
            "use Acme\\{};",
            "use function Acme\\makeValue;",
            "use const Acme\\LIMIT;",
            "function nested(): void {\n    use Acme\\Model\\User;\n}",
        )) {
            assertFalse(source, lex(source).any { it.type == DoriaTokenTypes.IMPORT_USE_KEYWORD })
        }
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
