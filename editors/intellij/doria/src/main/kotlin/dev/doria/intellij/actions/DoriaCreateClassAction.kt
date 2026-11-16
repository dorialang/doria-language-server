package dev.doria.intellij.actions

import com.intellij.icons.AllIcons
import com.intellij.ide.actions.CreateFileFromTemplateAction
import com.intellij.ide.fileTemplates.FileTemplateManager
import com.intellij.openapi.actionSystem.ActionUpdateThread
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.actionSystem.LangDataKeys
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.application.ModalityState
import com.intellij.openapi.command.WriteCommandAction
import com.intellij.openapi.project.DumbAwareAction
import com.intellij.openapi.project.Project
import com.intellij.openapi.ui.DialogWrapper
import com.intellij.openapi.ui.Messages
import com.intellij.openapi.ui.ComboBox
import com.intellij.openapi.ui.ValidationInfo
import com.intellij.openapi.ui.popup.JBPopupFactory
import com.intellij.psi.PsiDirectory
import com.intellij.psi.PsiFile
import com.intellij.ui.DocumentAdapter
import com.intellij.ui.SimpleListCellRenderer
import com.intellij.ui.TitledSeparator
import com.intellij.ui.components.JBLabel
import com.intellij.ui.components.JBList
import com.intellij.ui.components.JBScrollPane
import com.intellij.ui.components.JBTextField
import com.intellij.util.IncorrectOperationException
import com.intellij.util.Alarm
import com.intellij.util.PathUtilRt
import com.intellij.util.ui.UIUtil
import dev.doria.intellij.DoriaIcons
import java.awt.BorderLayout
import java.awt.Dimension
import java.awt.FlowLayout
import java.awt.GridBagConstraints
import java.awt.GridBagLayout
import java.awt.Insets
import javax.swing.DefaultComboBoxModel
import javax.swing.DefaultListModel
import javax.swing.JButton
import javax.swing.JComponent
import javax.swing.JComboBox
import javax.swing.JPanel
import javax.swing.ListSelectionModel
import javax.swing.event.DocumentEvent
import javax.swing.text.JTextComponent

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
    private val placement = DoriaAutoloadNamespaceResolver.placement(directory.virtualFile)
    private val classNameField = JBTextField(42)
    private val namespaceField = JComboBox(listOfNotNull(placement?.namespace).toTypedArray()).apply {
        isEditable = true
    }
    private val fileNameField = JBTextField(42)
    private val directoryField = JBTextField(directory.virtualFile.presentableUrl, 42)
    private val templateSelector = JComboBox(DoriaDeclarationTemplate.entries.toTypedArray())
    private val parentModel = DefaultComboBoxModel<DoriaSupertype?>()
    private val parentField = ComboBox(parentModel).apply {
        renderer = SimpleListCellRenderer.create { label, value, _ ->
            label.text = value?.headerText ?: "None"
            label.toolTipText = value?.qualifiedName
        }
    }
    private val interfaceModel = DefaultListModel<DoriaSupertype>()
    private val interfaceList = JBList(interfaceModel).apply {
        cellRenderer = SimpleListCellRenderer.create { label, value, _ ->
            label.text = value.headerText
            label.toolTipText = value.qualifiedName
        }
    }
    private val addInterfaceButton = JButton(AllIcons.General.Add)
    private val removeInterfaceButton = JButton(AllIcons.General.Remove)
    private val parentTypeStatus = JBLabel().apply {
        componentStyle = UIUtil.ComponentStyle.SMALL
        foreground = UIUtil.getContextHelpForeground()
    }
    private val namespaceAlarm = Alarm(Alarm.ThreadToUse.SWING_THREAD, disposable)
    private var lookup: DoriaSupertypeLookup? = null
    private var lookupGeneration = 0
    private var updatingFileName = false
    private var fileNameWasEdited = false

    val typeName: String
        get() = classNameField.text.trim()

    val template: DoriaDeclarationTemplate
        get() = templateSelector.selectedItem as? DoriaDeclarationTemplate
            ?: DoriaDeclarationTemplate.CLASS

    val fileBaseName: String
        get() = fileNameField.text.trim().removeSuffix(".doria")

    private val namespace: String
        get() = namespaceField.editor.item?.toString()?.trim().orEmpty()

    val namespaceDeclaration: String
        get() = namespace.let { namespace ->
            if (namespace.isEmpty()) "" else "namespace $namespace;\n\n"
        }

    val inheritanceClause: String
        get() = buildString {
            if (!template.supportsClassInheritance) return@buildString
            (parentField.selectedItem as? DoriaSupertype)?.let { append(" extends ").append(it.headerText) }

            val interfaces = interfaces()
            if (interfaces.isNotEmpty()) {
                append(" implements ").append(interfaces.joinToString(", ", transform = DoriaSupertype::headerText))
            }
        }

    /** The language server's answer, once it has one. */
    private val candidates: DoriaSupertypeCandidates?
        get() = (lookup as? DoriaSupertypeLookup.Found)?.candidates

    init {
        title = "Create New Doria Class"
        setOKButtonText("OK")
        directoryField.isEditable = false
        classNameField.emptyText.text = "Type name"
        namespaceField.toolTipText = "Namespace inferred from the nearest Baton.toml autoload mapping"
        fileNameField.emptyText.text = "TypeName.doria"
        parentField.toolTipText = "Open classes this package may extend"
        interfaceList.emptyText.text = "Choose interfaces to implement"
        interfaceList.selectionMode = ListSelectionModel.SINGLE_SELECTION
        interfaceList.addListSelectionListener { updateTemplateControls() }
        addInterfaceButton.toolTipText = "Add interfaces"
        addInterfaceButton.accessibleContext.accessibleName = "Add interfaces"
        addInterfaceButton.addActionListener { chooseInterfaces() }
        removeInterfaceButton.toolTipText = "Remove selected interface"
        removeInterfaceButton.accessibleContext.accessibleName = "Remove selected interface"
        removeInterfaceButton.addActionListener { removeSelectedInterface() }
        templateSelector.addActionListener { updateTemplateControls() }
        // Spellings depend on the namespace the new type is declared in.
        (namespaceField.editor.editorComponent as? JTextComponent)?.document?.addDocumentListener(
            object : DocumentAdapter() {
                override fun textChanged(event: DocumentEvent) {
                    namespaceAlarm.cancelAllRequests()
                    namespaceAlarm.addRequest(::requestSupertypes, NAMESPACE_SETTLE_MS)
                }
            },
        )
        namespaceField.addActionListener { requestSupertypes() }
        installFileNameSynchronization()
        init()
        requestSupertypes()
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
        add(
            parentTypeStatus,
            GridBagConstraints().apply {
                gridx = 1
                gridy = 9
                weightx = 1.0
                fill = GridBagConstraints.HORIZONTAL
                insets = Insets(0, 0, 4, 0)
            },
        )
    }

    override fun getPreferredFocusedComponent(): JComponent = classNameField

    override fun getInitialSize(): Dimension = Dimension(720, 560)

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
        if (namespace.isNotEmpty() && !isDoriaNamespaceName(namespace)) {
            return ValidationInfo("Enter a valid Doria namespace.", namespaceField)
        }
        return null
    }

    /** Asks the language server for the types this declaration may name. */
    private fun requestSupertypes() {
        val generation = ++lookupGeneration
        val requestedNamespace = namespace
        val fileName = "${fileBaseName.ifEmpty { "NewType" }}.doria"
        lookup = null
        parentTypeStatus.text = "Loading parent types from the Doria language server…"
        updateTemplateControls()
        ApplicationManager.getApplication().executeOnPooledThread {
            val result = DoriaSupertypeSource.lookup(
                project,
                directory.virtualFile,
                fileName,
                requestedNamespace,
                placement?.development ?: false,
            )
            // The dialog is modal; its own updates must run while it is open.
            ApplicationManager.getApplication().invokeLater(
                { if (!isDisposed && generation == lookupGeneration) showSupertypes(result) },
                ModalityState.any(),
            )
        }
    }

    private fun showSupertypes(result: DoriaSupertypeLookup) {
        lookup = result
        val selectedParent = (parentField.selectedItem as? DoriaSupertype)?.qualifiedName
        val selectedInterfaces = interfaces().map(DoriaSupertype::qualifiedName)
        parentModel.removeAllElements()
        parentModel.addElement(null)
        interfaceModel.clear()
        when (result) {
            is DoriaSupertypeLookup.Unavailable -> parentTypeStatus.text = result.reason
            is DoriaSupertypeLookup.Found -> {
                val candidates = result.candidates
                candidates.classes.forEach(parentModel::addElement)
                // Keep the author's choices, respelled for the current namespace.
                parentModel.selectedItem = candidates.classes.firstOrNull { it.qualifiedName == selectedParent }
                selectedInterfaces
                    .mapNotNull { name -> candidates.interfaces.firstOrNull { it.qualifiedName == name } }
                    .forEach(interfaceModel::addElement)
                parentTypeStatus.text = when {
                    candidates.packageName == null ->
                        "This directory is in no Baton package, so no parent types are listed. " +
                            "Create the type under a package's autoload directory to choose them."
                    else -> ""
                }
            }
        }
        updateTemplateControls()
    }

    private fun updateTemplateControls() {
        val enabled = template.supportsClassInheritance
        val candidates = candidates
        parentField.isEnabled = enabled && candidates != null && candidates.classes.isNotEmpty()
        interfaceList.isEnabled = enabled && candidates != null
        addInterfaceButton.isEnabled = enabled && candidates != null &&
            candidates.interfaces.any { it !in interfaces() }
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

    private fun chooseInterfaces() {
        val selected = interfaces()
        val available = candidates?.interfaces.orEmpty().filter { it !in selected }
        if (available.isEmpty()) return
        JBPopupFactory.getInstance()
            .createPopupChooserBuilder(available)
            .setTitle("Choose Interfaces")
            .setSelectionMode(ListSelectionModel.MULTIPLE_INTERVAL_SELECTION)
            .setRenderer(
                SimpleListCellRenderer.create { label, value: DoriaSupertype, _ ->
                    label.text = value.headerText
                    label.toolTipText = value.qualifiedName
                },
            )
            .setNamerForFiltering(DoriaSupertype::label)
            .setItemsChosenCallback { chosen ->
                chosen.forEach(interfaceModel::addElement)
                interfaceList.selectedIndex = interfaceModel.size() - 1
                updateTemplateControls()
            }
            .createPopup()
            .showUnderneathOf(addInterfaceButton)
    }

    private fun removeSelectedInterface() {
        val index = interfaceList.selectedIndex
        if (index >= 0) interfaceModel.remove(index)
        updateTemplateControls()
    }

    private fun interfaces(): List<DoriaSupertype> =
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

    /** A word the compiler's lexer reads as an identifier. Keywords come from the language server. */
    private fun isDoriaIdentifier(value: String): Boolean =
        DORIA_IDENTIFIER.matches(value) && candidates?.keywords?.contains(value) != true

    private fun isDoriaClassName(value: String): Boolean =
        isDoriaIdentifier(value) &&
            value !in DORIA_RESERVED_CLASS_NAMES &&
            !value.equals("__DoriaDisplayable", ignoreCase = true)

    private fun isDoriaNamespaceName(value: String): Boolean =
        DORIA_QUALIFIED_NAME.matches(value) && value.split('\\').all(::isDoriaIdentifier)

    private companion object {
        const val NAMESPACE_SETTLE_MS = 300
        val DORIA_IDENTIFIER = Regex("[A-Za-z_][A-Za-z0-9_]*")
        val DORIA_QUALIFIED_NAME = Regex("""[A-Za-z_][A-Za-z0-9_]*(?:\\[A-Za-z_][A-Za-z0-9_]*)*""")
        val DORIA_RESERVED_CLASS_NAMES = setOf(
            "Int", "Int8", "Int16", "Int32", "Int64", "UInt8", "UInt16", "UInt32", "UInt64",
            "Float", "Float32", "Float64", "Bool", "Displayable",
        )
    }
}
