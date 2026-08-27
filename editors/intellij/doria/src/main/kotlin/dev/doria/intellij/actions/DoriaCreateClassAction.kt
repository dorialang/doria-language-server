package dev.doria.intellij.actions

import com.intellij.icons.AllIcons
import com.intellij.ide.actions.CreateFileFromTemplateAction
import com.intellij.ide.fileTemplates.FileTemplateManager
import com.intellij.openapi.actionSystem.ActionUpdateThread
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.actionSystem.LangDataKeys
import com.intellij.openapi.command.WriteCommandAction
import com.intellij.openapi.project.DumbAwareAction
import com.intellij.openapi.project.Project
import com.intellij.openapi.ui.DialogWrapper
import com.intellij.openapi.ui.Messages
import com.intellij.openapi.ui.ValidationInfo
import com.intellij.psi.PsiDirectory
import com.intellij.psi.PsiFile
import com.intellij.ui.DocumentAdapter
import com.intellij.ui.TitledSeparator
import com.intellij.ui.components.JBLabel
import com.intellij.ui.components.JBList
import com.intellij.ui.components.JBScrollPane
import com.intellij.ui.components.JBTextField
import com.intellij.util.IncorrectOperationException
import com.intellij.util.PathUtilRt
import dev.doria.intellij.DoriaIcons
import java.awt.BorderLayout
import java.awt.Dimension
import java.awt.FlowLayout
import java.awt.GridBagConstraints
import java.awt.GridBagLayout
import java.awt.Insets
import javax.swing.DefaultListModel
import javax.swing.JButton
import javax.swing.JComponent
import javax.swing.JComboBox
import javax.swing.JPanel
import javax.swing.ListSelectionModel
import javax.swing.event.DocumentEvent

class DoriaCreateClassAction : DumbAwareAction(
    "Doria Class",
    "Create a Doria class",
    DoriaIcons.FILE,
) {
    override fun getActionUpdateThread(): ActionUpdateThread = ActionUpdateThread.EDT

    override fun update(event: AnActionEvent) {
        event.presentation.isEnabledAndVisible =
            event.project != null && event.getData(LangDataKeys.IDE_VIEW) != null
    }

    override fun actionPerformed(event: AnActionEvent) {
        val project = event.project ?: return
        val view = event.getData(LangDataKeys.IDE_VIEW) ?: return
        val directory = view.orChooseDirectory ?: return
        val dialog = DoriaCreateClassDialog(project, directory)
        if (!dialog.showAndGet()) return

        try {
            val created = WriteCommandAction.writeCommandAction(project)
                .withName("Create Doria ${dialog.template.displayName.lowercase()} ${dialog.typeName}")
                .compute<PsiFile?, RuntimeException> {
                    createFromTemplate(project, directory, dialog)
                }
            created?.let(view::selectElement)
        } catch (error: IncorrectOperationException) {
            Messages.showErrorDialog(
                project,
                error.message ?: "The Doria declaration could not be created.",
                "Cannot Create Doria Declaration",
            )
        }
    }

    private fun createFromTemplate(
        project: Project,
        directory: PsiDirectory,
        dialog: DoriaCreateClassDialog,
    ): PsiFile? {
        val template = FileTemplateManager.getInstance(project)
            .getInternalTemplate(dialog.template.fileTemplateName)
        return CreateFileFromTemplateAction.createFileFromTemplate(
            dialog.fileBaseName,
            template,
            directory,
            null,
            true,
            emptyMap(),
            mapOf(
                "NAMESPACE_DECLARATION" to dialog.namespaceDeclaration,
                "TYPE_NAME" to dialog.typeName,
                "INHERITANCE" to dialog.inheritanceClause,
            ),
        )
    }
}

internal enum class DoriaDeclarationTemplate(
    val displayName: String,
    val fileTemplateName: String,
    val supportsClassInheritance: Boolean,
) {
    CLASS("Class", "Doria Class", true),
    INTERFACE("Interface", "Doria Interface", false),
    TRAIT("Trait", "Doria Trait", false),
    ENUM("Enum", "Doria Enum", false),
    ;

    override fun toString(): String = displayName
}

private class DoriaCreateClassDialog(
    private val project: Project,
    private val directory: PsiDirectory,
) : DialogWrapper(project, true) {
    private val classNameField = JBTextField(42)
    private val namespaceField = JComboBox(
        listOfNotNull(DoriaAutoloadNamespaceResolver.suggest(directory.virtualFile)).toTypedArray(),
    ).apply {
        isEditable = true
    }
    private val fileNameField = JBTextField(42)
    private val directoryField = JBTextField(directory.virtualFile.presentableUrl, 42)
    private val templateSelector = JComboBox(DoriaDeclarationTemplate.entries.toTypedArray())
    private val parentField = JBTextField(42)
    private val interfaceModel = DefaultListModel<String>()
    private val interfaceList = JBList(interfaceModel)
    private val addInterfaceButton = JButton(AllIcons.General.Add)
    private val removeInterfaceButton = JButton(AllIcons.General.Remove)
    private var updatingFileName = false
    private var fileNameWasEdited = false

    val typeName: String
        get() = classNameField.text.trim()

    val template: DoriaDeclarationTemplate
        get() = templateSelector.selectedItem as? DoriaDeclarationTemplate
            ?: DoriaDeclarationTemplate.CLASS

    val fileBaseName: String
        get() = fileNameField.text.trim().removeSuffix(".doria")

    val namespaceDeclaration: String
        get() = namespaceField.editor.item?.toString()?.trim().orEmpty().let { namespace ->
            if (namespace.isEmpty()) "" else "namespace $namespace;\n\n"
        }

    val inheritanceClause: String
        get() = buildString {
            if (!template.supportsClassInheritance) return@buildString
            val parent = parentField.text.trim()
            if (parent.isNotEmpty()) append(" extends ").append(parent)

            val interfaces = interfaces()
            if (interfaces.isNotEmpty()) {
                append(" implements ").append(interfaces.joinToString(", "))
            }
        }

    init {
        title = "Create New Doria Class"
        setOKButtonText("OK")
        directoryField.isEditable = false
        classNameField.emptyText.text = "Type name"
        namespaceField.toolTipText = "Namespace inferred from the nearest Baton.toml autoload mapping"
        fileNameField.emptyText.text = "TypeName.doria"
        parentField.emptyText.text = "Optional parent class"
        interfaceList.emptyText.text = "Choose interfaces to implement"
        interfaceList.selectionMode = ListSelectionModel.SINGLE_SELECTION
        interfaceList.addListSelectionListener {
            removeInterfaceButton.isEnabled = interfaceList.selectedIndex >= 0
        }
        addInterfaceButton.toolTipText = "Add interface"
        addInterfaceButton.accessibleContext.accessibleName = "Add interface"
        addInterfaceButton.addActionListener { addInterface() }
        removeInterfaceButton.toolTipText = "Remove selected interface"
        removeInterfaceButton.accessibleContext.accessibleName = "Remove selected interface"
        removeInterfaceButton.isEnabled = false
        removeInterfaceButton.addActionListener { removeSelectedInterface() }
        templateSelector.addActionListener { updateTemplateControls() }
        installFileNameSynchronization()
        updateTemplateControls()
        init()
    }

    override fun createCenterPanel(): JComponent = JPanel(GridBagLayout()).apply {
        addSection(this, 0, "Doria type")
        addRow(this, 1, "Name:", classNameField)
        addRow(this, 2, "Namespace:", namespaceField)
        addRow(this, 3, "File name:", fileNameField)
        addRow(this, 4, "Directory:", directoryField)
        addRow(this, 5, "Template:", templateSelector)
        addSection(this, 6, "Parent types")
        addRow(this, 7, "Extends:", parentField)

        add(
            JBLabel("Implements:"),
            GridBagConstraints().apply {
                gridx = 0
                gridy = 8
                anchor = GridBagConstraints.FIRST_LINE_START
                insets = Insets(8, 20, 4, 12)
            },
        )
        add(
            interfacePanel(),
            GridBagConstraints().apply {
                gridx = 1
                gridy = 8
                weightx = 1.0
                weighty = 1.0
                fill = GridBagConstraints.BOTH
                insets = Insets(4, 0, 4, 0)
            },
        )
    }

    override fun getPreferredFocusedComponent(): JComponent = classNameField

    override fun getInitialSize(): Dimension = Dimension(720, 540)

    override fun doValidate(): ValidationInfo? {
        if (!isDoriaClassName(typeName)) {
            return ValidationInfo("Enter a valid Doria type name.", classNameField)
        }
        if (fileBaseName.isEmpty() || !PathUtilRt.isValidFileName("$fileBaseName.doria", true)) {
            return ValidationInfo("Enter a valid Doria file name.", fileNameField)
        }
        if (directory.findFile("$fileBaseName.doria") != null) {
            return ValidationInfo("$fileBaseName.doria already exists.", fileNameField)
        }

        val namespace = namespaceField.editor.item?.toString()?.trim().orEmpty()
        if (namespace.isNotEmpty() && !isDoriaNamespaceName(namespace)) {
            return ValidationInfo("Enter a valid Doria namespace.", namespaceField)
        }
        if (template.supportsClassInheritance) {
            val parent = parentField.text.trim()
            if (parent.isNotEmpty() && !isDoriaQualifiedClassName(parent)) {
                return ValidationInfo("Enter one valid Doria parent type.", parentField)
            }
        }
        return null
    }

    private fun updateTemplateControls() {
        val enabled = template.supportsClassInheritance
        parentField.isEnabled = enabled
        interfaceList.isEnabled = enabled
        addInterfaceButton.isEnabled = enabled
        removeInterfaceButton.isEnabled = enabled && interfaceList.selectedIndex >= 0
    }

    private fun installFileNameSynchronization() {
        classNameField.document.addDocumentListener(object : DocumentAdapter() {
            override fun textChanged(event: DocumentEvent) {
                if (fileNameWasEdited) return
                updatingFileName = true
                val name = classNameField.text.trim()
                fileNameField.text = if (name.isEmpty()) "" else "$name.doria"
                updatingFileName = false
            }
        })
        fileNameField.document.addDocumentListener(object : DocumentAdapter() {
            override fun textChanged(event: DocumentEvent) {
                if (!updatingFileName) fileNameWasEdited = true
            }
        })
    }

    private fun addInterface() {
        val interfaceName = Messages.showInputDialog(
            project,
            "Interface name:",
            "Add Doria Interface",
            DoriaIcons.FILE,
        )?.trim() ?: return
        if (!isDoriaQualifiedInterfaceName(interfaceName)) {
            Messages.showErrorDialog(
                project,
                "Enter a valid Doria interface type.",
                "Invalid Interface",
            )
            return
        }
        if (interfaces().contains(interfaceName)) {
            Messages.showErrorDialog(
                project,
                "$interfaceName is already selected.",
                "Duplicate Interface",
            )
            return
        }
        interfaceModel.addElement(interfaceName)
        interfaceList.selectedIndex = interfaceModel.size() - 1
    }

    private fun removeSelectedInterface() {
        val index = interfaceList.selectedIndex
        if (index >= 0) interfaceModel.remove(index)
    }

    private fun interfaces(): List<String> =
        (0 until interfaceModel.size()).map(interfaceModel::getElementAt)

    private fun interfacePanel(): JComponent = JPanel(BorderLayout(0, 4)).apply {
        add(
            JPanel(FlowLayout(FlowLayout.LEFT, 0, 0)).apply {
                add(addInterfaceButton)
                add(removeInterfaceButton)
            },
            BorderLayout.NORTH,
        )
        add(JBScrollPane(interfaceList), BorderLayout.CENTER)
    }

    private fun addSection(panel: JPanel, row: Int, title: String) {
        panel.add(
            TitledSeparator(title),
            GridBagConstraints().apply {
                gridx = 0
                gridy = row
                gridwidth = 2
                weightx = 1.0
                fill = GridBagConstraints.HORIZONTAL
                insets = Insets(if (row == 0) 0 else 12, 0, 4, 0)
            },
        )
    }

    private fun addRow(panel: JPanel, row: Int, label: String, field: JComponent) {
        panel.add(
            JBLabel(label),
            GridBagConstraints().apply {
                gridx = 0
                gridy = row
                anchor = GridBagConstraints.LINE_START
                insets = Insets(4, 20, 4, 12)
            },
        )
        panel.add(
            field,
            GridBagConstraints().apply {
                gridx = 1
                gridy = row
                weightx = 1.0
                fill = GridBagConstraints.HORIZONTAL
                insets = Insets(4, 0, 4, 0)
            },
        )
    }

    private companion object {
        val DORIA_IDENTIFIER = Regex("[A-Za-z_][A-Za-z0-9_]*")
        val DORIA_QUALIFIED_NAME = Regex("""[A-Za-z_][A-Za-z0-9_]*(?:\\[A-Za-z_][A-Za-z0-9_]*)*""")
        val DORIA_RESERVED_NAME_SEGMENTS = setOf(
            "class", "interface", "implements", "namespace", "extends", "function",
            "internal", "static", "self", "parent", "const", "let", "take", "writable", "readonly", "return", "echo",
            "new", "foreach", "as", "if", "else", "while", "for", "break", "continue",
            "throw", "throws", "true", "false", "null", "void", "int", "int8", "int16",
            "int32", "int64", "uint8", "uint16", "uint32", "uint64", "float", "float32",
            "float64", "string", "bool", "not", "and", "or", "xor", "async", "await",
            "spawn", "scope", "trait", "enum", "match", "try", "catch", "mixed", "never",
            "resource", "array", "object", "use", "uses", "include", "declare", "with",
            "case", "when", "given", "finally", "unsafe", "extern", "open", "override", "is",
            "default", "do", "fn", "once", "get", "set", "insteadof", "shared",
            "goto", "require", "require_once", "include_once", "print", "instanceof",
        )
        val DORIA_RESERVED_CLASS_NAMES = setOf(
            "Int", "Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64",
            "Float", "Float32", "Float64", "Bool", "Displayable",
        )

        fun isDoriaIdentifier(value: String): Boolean =
            DORIA_IDENTIFIER.matches(value) && value !in DORIA_RESERVED_NAME_SEGMENTS

        fun isDoriaClassName(value: String): Boolean =
            isDoriaIdentifier(value) &&
                value !in DORIA_RESERVED_CLASS_NAMES &&
                !value.equals("__DoriaDisplayable", ignoreCase = true)

        fun isDoriaNamespaceName(value: String): Boolean =
            DORIA_QUALIFIED_NAME.matches(value) && value.split('\\').all(::isDoriaIdentifier)

        fun isDoriaQualifiedClassName(value: String): Boolean {
            if (!DORIA_QUALIFIED_NAME.matches(value)) return false
            val segments = value.split('\\')
            return segments.dropLast(1).all(::isDoriaIdentifier) && isDoriaClassName(segments.last())
        }

        fun isDoriaQualifiedInterfaceName(value: String): Boolean =
            value == "Displayable" || isDoriaQualifiedClassName(value)
    }
}
