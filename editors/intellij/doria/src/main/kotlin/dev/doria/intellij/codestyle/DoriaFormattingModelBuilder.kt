package dev.doria.intellij.codestyle

import com.intellij.formatting.Alignment
import com.intellij.formatting.Block
import com.intellij.formatting.ChildAttributes
import com.intellij.formatting.FormattingContext
import com.intellij.formatting.FormattingModel
import com.intellij.formatting.FormattingModelBuilder
import com.intellij.formatting.FormattingModelProvider
import com.intellij.formatting.Indent
import com.intellij.formatting.Spacing
import com.intellij.formatting.Wrap
import com.intellij.lang.ASTNode
import com.intellij.psi.TokenType
import com.intellij.psi.codeStyle.CommonCodeStyleSettings
import com.intellij.psi.formatter.common.AbstractBlock
import dev.doria.intellij.DoriaLanguage
import dev.doria.intellij.highlighting.DoriaTokenTypes
import dev.doria.intellij.psi.DoriaElementTypes

class DoriaFormattingModelBuilder : FormattingModelBuilder {
    override fun createModel(formattingContext: FormattingContext): FormattingModel {
        val settings = formattingContext.codeStyleSettings
        val commonSettings = settings.getCommonSettings(DoriaLanguage)
        val indentOptions = settings.getLanguageIndentOptions(DoriaLanguage)
        val rootBlock = DoriaFormattingBlock(
            formattingContext.node,
            null,
            null,
            commonSettings,
            indentOptions,
            null,
        )
        return FormattingModelProvider.createFormattingModelForPsiFile(
            formattingContext.containingFile,
            rootBlock,
            settings,
        )
    }
}

private class DoriaFormattingBlock(
    node: ASTNode,
    wrap: Wrap?,
    alignment: Alignment?,
    private val commonSettings: CommonCodeStyleSettings,
    private val indentOptions: CommonCodeStyleSettings.IndentOptions,
    private val parentBlock: DoriaFormattingBlock?,
) : AbstractBlock(node, wrap, alignment) {
    override fun buildChildren(): List<Block> {
        val blocks = mutableListOf<Block>()
        var child = myNode.firstChildNode
        while (child != null) {
            if (child.elementType != TokenType.WHITE_SPACE) {
                blocks += DoriaFormattingBlock(
                    child,
                    null,
                    null,
                    commonSettings,
                    indentOptions,
                    this,
                )
            }
            child = child.treeNext
        }
        return blocks
    }

    override fun getIndent(): Indent {
        val columns = absoluteIndentColumns(ownDelimiter = isOwnDelimiter())
        return if (columns == 0) Indent.getAbsoluteNoneIndent() else Indent.getSpaceIndent(columns)
    }

    override fun getSpacing(child1: Block?, child2: Block): Spacing? {
        val left = child1 as? DoriaFormattingBlock ?: return null
        val right = child2 as? DoriaFormattingBlock ?: return null
        return DoriaSpacing.create(left, right, commonSettings)
    }

    override fun getChildAttributes(newChildIndex: Int): ChildAttributes {
        val children = subBlocks
        val nextText = children.getOrNull(newChildIndex)
            ?.let { it as? DoriaFormattingBlock }
            ?.firstLeafText()
        val baseColumns = absoluteIndentColumns(ownDelimiter = false)
        val indentColumns = when (myNode.elementType) {
            DoriaElementTypes.BLOCK ->
                if (nextText == "}") baseColumns else baseColumns + indentOptions.INDENT_SIZE

            DoriaElementTypes.PARENTHESIZED,
            DoriaElementTypes.BRACKETED,
            -> if (nextText == ")" || nextText == "]") {
                baseColumns
            } else {
                baseColumns + indentOptions.CONTINUATION_INDENT_SIZE
            }

            else -> baseColumns
        }
        val indent = if (indentColumns == 0) {
            Indent.getAbsoluteNoneIndent()
        } else {
            Indent.getSpaceIndent(indentColumns)
        }
        return ChildAttributes(indent, null)
    }

    override fun isLeaf(): Boolean = myNode.firstChildNode == null

    fun firstLeafText(): String = generateSequence(myNode) { it.firstChildNode }
        .last()
        .text

    fun lastLeafText(): String = generateSequence(myNode) { it.lastChildNode }
        .last()
        .text

    fun firstLeafType() = generateSequence(myNode) { it.firstChildNode }
        .last()
        .elementType

    fun lastLeafType() = generateSequence(myNode) { it.lastChildNode }
        .last()
        .elementType

    fun parentTextBefore(): String {
        val parent = myNode.treeParent ?: return ""
        val relativeOffset = (myNode.startOffset - parent.startOffset).coerceIn(0, parent.textLength)
        return parent.text.substring(0, relativeOffset)
    }

    fun enclosingGroupPrefix(): String = parentBlock?.parentTextBefore().orEmpty()

    private fun absoluteIndentColumns(ownDelimiter: Boolean): Int {
        var blockDepth = 0
        var continuationDepth = 0
        var ancestor = parentBlock
        while (ancestor != null) {
            when (ancestor.myNode.elementType) {
                DoriaElementTypes.BLOCK -> blockDepth++
                DoriaElementTypes.PARENTHESIZED,
                DoriaElementTypes.BRACKETED,
                -> continuationDepth++
            }
            ancestor = ancestor.parentBlock
        }

        if (ownDelimiter) {
            when (firstLeafText()) {
                "{", "}" -> blockDepth--
                "(", ")", "[", "]" -> continuationDepth--
            }
        }

        return blockDepth.coerceAtLeast(0) * indentOptions.INDENT_SIZE +
            continuationDepth.coerceAtLeast(0) * indentOptions.CONTINUATION_INDENT_SIZE
    }

    private fun isOwnDelimiter(): Boolean = firstLeafText() in setOf("{", "}", "(", ")", "[", "]")
}

private object DoriaSpacing {
    private val assignmentOperators = setOf("=", "+=", "-=", "*=", "/=", "%=", "??=")
    private val logicalOperators = setOf("&&", "||")
    private val wordLogicalOperators = setOf("and", "or", "xor")
    private val equalityOperators = setOf("==", "!=")
    private val relationalOperators = setOf("<", ">", "<=", ">=")
    private val typeTestOperators = setOf("is")
    private val bitwiseOperators = setOf("&", "|", "^")
    private val additiveOperators = setOf("+", "-", ".")
    private val multiplicativeOperators = setOf("*", "/", "%")
    private val shiftOperators = setOf("<<", ">>", "<<=", ">>=")
    private val controlKeywords = setOf("if", "for", "foreach", "while", "catch", "given", "when")
    private val declarationKeywords = setOf("class", "interface", "trait", "enum")

    fun create(
        left: DoriaFormattingBlock,
        right: DoriaFormattingBlock,
        settings: CommonCodeStyleSettings,
    ): Spacing {
        val leftText = left.lastLeafText()
        val rightText = right.firstLeafText()
        val keepLineBreaks = settings.KEEP_LINE_BREAKS
        val keepBlankLines = maxOf(
            settings.KEEP_BLANK_LINES_IN_CODE,
            settings.KEEP_BLANK_LINES_IN_DECLARATIONS,
            settings.KEEP_BLANK_LINES_BEFORE_RBRACE,
        )

        fun spacing(
            spaces: Int,
            lineFeeds: Int = 0,
            preserveLineBreaks: Boolean = keepLineBreaks,
        ): Spacing = Spacing.createSpacing(
            spaces,
            spaces,
            lineFeeds,
            preserveLineBreaks,
            keepBlankLines,
        )

        if (left.lastLeafType() in stringTokens || right.firstLeafType() in stringTokens) {
            return Spacing.getReadOnlySpacing()
        }
        if (left.lastLeafType() == DoriaTokenTypes.ATTRIBUTE_DELIMITER ||
            right.firstLeafType() == DoriaTokenTypes.ATTRIBUTE_DELIMITER
        ) {
            return spacing(0)
        }
        if (left.lastLeafType() in setOf(DoriaTokenTypes.COMMENT, DoriaTokenTypes.DOC_COMMENT)) {
            return spacing(0, 1)
        }
        if (rightText == ",") return spacing(if (settings.SPACE_BEFORE_COMMA) 1 else 0)
        if (rightText == ";") return spacing(if (settings.SPACE_BEFORE_SEMICOLON) 1 else 0)
        if (rightText == ":") return spacing(0)
        if (leftText == ",") return spacing(if (settings.SPACE_AFTER_COMMA) 1 else 0)
        if (leftText == ";") return spacing(if (settings.SPACE_AFTER_SEMICOLON) 1 else 0)
        if (leftText == ":") return spacing(1)
        if (leftText in tightOperators || rightText in tightOperators) return spacing(0)
        if (rightText == "[" && leftText !in setOf("=", ",", "(", "[")) return spacing(0)
        if (isGenericTypeBoundary(left, right)) return spacing(0)
        if (leftText in unaryOperators || rightText in unaryOperators) {
            return spacing(if (settings.SPACE_AROUND_UNARY_OPERATOR) 1 else 0)
        }
        if (leftText == "?" && right.firstLeafType() in typeTokens) return spacing(0)

        if (rightText == "(") {
            val before = when {
                leftText in controlKeywords -> controlParenthesisSpacing(leftText, settings)
                left.lastLeafType() == DoriaTokenTypes.FUNCTION_DECLARATION ->
                    settings.SPACE_BEFORE_METHOD_PARENTHESES

                else -> settings.SPACE_BEFORE_METHOD_CALL_PARENTHESES
            }
            return spacing(if (before) 1 else 0)
        }
        if (leftText == "(" || rightText == ")") {
            return spacing(if (spaceWithinParentheses(left, right, settings)) 1 else 0)
        }
        if (leftText == "[" || rightText == "]") {
            return spacing(if (settings.SPACE_WITHIN_BRACKETS) 1 else 0)
        }

        if (rightText == "{") {
            val braceStyle = braceStyleBefore(right, settings)
            val lineFeeds = if (braceStyle == CommonCodeStyleSettings.END_OF_LINE) 0 else 1
            return spacing(
                if (lineFeeds == 0 && spaceBeforeBrace(right, settings)) 1 else 0,
                lineFeeds,
                preserveLineBreaks = false,
            )
        }
        if ((leftText == "}" || (leftText == ")" && rightText == "finally")) &&
            rightText in setOf("else", "catch", "finally")
        ) {
            val newLine = when (rightText) {
                "else" -> settings.ELSE_ON_NEW_LINE
                "catch" -> settings.CATCH_ON_NEW_LINE
                else -> settings.FINALLY_ON_NEW_LINE
            }
            return spacing(if (newLine) 0 else 1, if (newLine) 1 else 0, preserveLineBreaks = false)
        }

        val operator = when {
            leftText in allBinaryOperators -> leftText
            rightText in allBinaryOperators -> rightText
            else -> null
        }
        if (operator != null) {
            return spacing(if (spaceAround(operator, settings)) 1 else 0)
        }

        return spacing(1)
    }

    private val allBinaryOperators = assignmentOperators + logicalOperators + wordLogicalOperators +
        equalityOperators + relationalOperators + typeTestOperators + bitwiseOperators +
        additiveOperators + multiplicativeOperators + shiftOperators
    private val tightOperators = setOf("\\", "->", "?->", "::", "#")
    private val unaryOperators = setOf("!", "~", "++", "--")
    private val stringTokens = setOf(DoriaTokenTypes.STRING, DoriaTokenTypes.ESCAPE_SEQUENCE)
    private val typeTokens = setOf(
        DoriaTokenTypes.PRIMITIVE_TYPE,
        DoriaTokenTypes.RESERVED_TYPE,
        DoriaTokenTypes.COLLECTION_TYPE,
        DoriaTokenTypes.TYPE_NAME,
    )

    private fun isGenericTypeBoundary(
        left: DoriaFormattingBlock,
        right: DoriaFormattingBlock,
    ): Boolean {
        val leftText = left.lastLeafText()
        val rightText = right.firstLeafText()
        return (rightText == "<" && looksLikeType(left)) ||
            (leftText == "<" && looksLikeType(right)) ||
            (rightText.isGenericClose() && looksLikeType(left)) ||
            (leftText == ">" && rightText in setOf(">", ",", "?", "[", ")"))
    }

    private fun String.isGenericClose(): Boolean = isNotEmpty() && all { it == '>' }

    private fun looksLikeType(block: DoriaFormattingBlock): Boolean {
        val text = block.firstLeafText()
        return block.firstLeafType() in typeTokens || text in builtinTypeNames || text.firstOrNull()?.isUpperCase() == true
    }

    private val builtinTypeNames = setOf(
        "void", "int", "int8", "int16", "int32", "int64", "uint8", "uint16", "uint32", "uint64",
        "float", "float32", "float64", "string", "bool", "mixed",
    )

    private fun spaceAround(operator: String, settings: CommonCodeStyleSettings): Boolean = when (operator) {
        in assignmentOperators -> settings.SPACE_AROUND_ASSIGNMENT_OPERATORS
        in logicalOperators -> settings.SPACE_AROUND_LOGICAL_OPERATORS
        in wordLogicalOperators, in typeTestOperators -> true
        in equalityOperators -> settings.SPACE_AROUND_EQUALITY_OPERATORS
        in relationalOperators -> settings.SPACE_AROUND_RELATIONAL_OPERATORS
        in bitwiseOperators -> settings.SPACE_AROUND_BITWISE_OPERATORS
        in additiveOperators -> settings.SPACE_AROUND_ADDITIVE_OPERATORS
        in multiplicativeOperators -> settings.SPACE_AROUND_MULTIPLICATIVE_OPERATORS
        in shiftOperators -> settings.SPACE_AROUND_SHIFT_OPERATORS
        else -> true
    }

    private fun controlParenthesisSpacing(
        keyword: String,
        settings: CommonCodeStyleSettings,
    ): Boolean = when (keyword) {
        "if", "given", "when" -> settings.SPACE_BEFORE_IF_PARENTHESES
        "while" -> settings.SPACE_BEFORE_WHILE_PARENTHESES
        "for", "foreach" -> settings.SPACE_BEFORE_FOR_PARENTHESES
        "catch" -> settings.SPACE_BEFORE_CATCH_PARENTHESES
        else -> true
    }

    private fun spaceWithinParentheses(
        left: DoriaFormattingBlock,
        right: DoriaFormattingBlock,
        settings: CommonCodeStyleSettings,
    ): Boolean {
        val groupPrefix = if (left.lastLeafText() == "(") {
            left.enclosingGroupPrefix()
        } else {
            right.enclosingGroupPrefix()
        }.trimEnd()

        return when {
            Regex("\\bfunction\\s+[A-Za-z_][A-Za-z0-9_]*$").containsMatchIn(groupPrefix) ->
                settings.SPACE_WITHIN_METHOD_PARENTHESES

            Regex("\\b(if|given|when)$").containsMatchIn(groupPrefix) ->
                settings.SPACE_WITHIN_IF_PARENTHESES

            Regex("\\bwhile$").containsMatchIn(groupPrefix) ->
                settings.SPACE_WITHIN_WHILE_PARENTHESES

            Regex("\\b(for|foreach)$").containsMatchIn(groupPrefix) ->
                settings.SPACE_WITHIN_FOR_PARENTHESES

            Regex("\\bcatch$").containsMatchIn(groupPrefix) ->
                settings.SPACE_WITHIN_CATCH_PARENTHESES

            Regex("(?:[A-Za-z_][A-Za-z0-9_]*|\\)|\\])$").containsMatchIn(groupPrefix) ->
                settings.SPACE_WITHIN_METHOD_CALL_PARENTHESES

            else -> settings.SPACE_WITHIN_PARENTHESES
        }
    }

    private fun braceStyleBefore(
        right: DoriaFormattingBlock,
        settings: CommonCodeStyleSettings,
    ): Int {
        val prefix = right.parentTextBefore()
        val declaration = declarationKeywords.any { Regex("\\b$it\\b[^{};]*$").containsMatchIn(prefix) }
        val method = Regex("\\bfunction\\b[^{};]*$").containsMatchIn(prefix)
        return when {
            declaration -> settings.CLASS_BRACE_STYLE
            method -> settings.METHOD_BRACE_STYLE
            else -> settings.BRACE_STYLE
        }
    }

    private fun spaceBeforeBrace(
        right: DoriaFormattingBlock,
        settings: CommonCodeStyleSettings,
    ): Boolean {
        val prefix = right.parentTextBefore()
        return when {
            declarationKeywords.any { Regex("\\b$it\\b[^{};]*$").containsMatchIn(prefix) } ->
                settings.SPACE_BEFORE_CLASS_LBRACE

            Regex("\\bfunction\\b[^{};]*$").containsMatchIn(prefix) ->
                settings.SPACE_BEFORE_METHOD_LBRACE

            Regex("\\belse\\b[^{};]*$").containsMatchIn(prefix) -> settings.SPACE_BEFORE_ELSE_LBRACE
            Regex("\\bwhile\\b[^{};]*$").containsMatchIn(prefix) -> settings.SPACE_BEFORE_WHILE_LBRACE
            Regex("\\b(for|foreach)\\b[^{};]*$").containsMatchIn(prefix) -> settings.SPACE_BEFORE_FOR_LBRACE
            Regex("\\btry\\b[^{};]*$").containsMatchIn(prefix) -> settings.SPACE_BEFORE_TRY_LBRACE
            Regex("\\bcatch\\b[^{};]*$").containsMatchIn(prefix) -> settings.SPACE_BEFORE_CATCH_LBRACE
            Regex("\\bfinally\\b[^{};]*$").containsMatchIn(prefix) -> settings.SPACE_BEFORE_FINALLY_LBRACE
            else -> settings.SPACE_BEFORE_IF_LBRACE
        }
    }
}
