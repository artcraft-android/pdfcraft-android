package io.github.artcraftandroid.pdfcraft.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.artcraftandroid.pdfcraft.ui.home.HomeRoot
import io.github.artcraftandroid.pdfcraft.ui.home.SettingsScreen
import io.github.artcraftandroid.pdfcraft.ui.viewer.SignaturePadScreen
import io.github.artcraftandroid.pdfcraft.ui.viewer.ViewerScreen

/** Home or the viewer, and everything that can come over them: tools, settings, dialogs, snackbars. */
@Composable
fun AppRoot(vm: AppViewModel, onBrowse: () -> Unit, onPickSignatureImage: () -> Unit) {
    val c = pdfColors
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(vm) {
        for (m in vm.messages) {
            val result = snackbar.showSnackbar(
                m.text, actionLabel = m.actionLabel, withDismissAction = m.actionLabel == null,
                duration = if (m.actionLabel != null) SnackbarDuration.Long else SnackbarDuration.Short,
            )
            if (result == SnackbarResult.ActionPerformed) m.action?.invoke()
        }
    }
    val doc = vm.document
    // outside the viewer (which sets its own) the bar icons follow the app's theme, not the system's
    val view = androidx.compose.ui.platform.LocalView.current
    val activity = androidx.compose.ui.platform.LocalContext.current as? android.app.Activity
    androidx.compose.runtime.SideEffect {
        val window = activity?.window ?: return@SideEffect
        if (doc == null) {
            val insets = androidx.core.view.WindowCompat.getInsetsController(window, view)
            insets.isAppearanceLightStatusBars = !c.dark
            insets.isAppearanceLightNavigationBars = !c.dark
        }
    }
    // Back in the viewer closes the document (saving it) and returns to Home
    androidx.activity.compose.BackHandler(enabled = doc != null && vm.toolScreen == null) { vm.closeDocument() }
    // what floats above Home or the viewer (sheets, the snackbar, the viewer's bars) frosts the screen below it
    val glass = rememberGlassSource()
    androidx.compose.runtime.CompositionLocalProvider(LocalGlassSource provides glass) {
    // sideways, the camera cutout and the navigation bar sit at the sides: content stays between
    // them, on black, as the reference's does
    Box(
        Modifier.fillMaxSize().background(Color.Black)
            .windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Horizontal))
            .background(c.surface),
    ) {
        when {
            doc != null -> ViewerScreen(vm, doc)
            else -> HomeRoot(vm, onBrowse, glass)
        }
        // here rather than in the file's row: a sheet inside a scrolling list has no height to fill
        if (doc == null) vm.fileSheet?.let { (r, recent) -> io.github.artcraftandroid.pdfcraft.ui.home.FileSheet(vm, r, recent) { vm.fileSheet = null } }
        vm.toolScreen?.let { ToolScreenHost(vm, it) }
        vm.signaturePad?.let { SignaturePadScreen(vm, it, onPickSignatureImage) }
        if (vm.settingsOpen) SettingsScreen(vm)
        if (vm.showLicences) LicencesScreen(vm)

        if (vm.opening) CircularProgressIndicator(Modifier.align(Alignment.Center), color = c.accent)
        // clear of the viewer's floating bottom bubble as well as the home tab bar
        SnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).navigationBarsPadding().padding(bottom = 84.dp, start = FloatSide, end = FloatSide)) { GlassSnackbar(it) }

        vm.passwordRequest?.let { PasswordDialog(vm, it) }
        if (vm.authorPrompt) {
            var name by remember { mutableStateOf("") }
            PdfDialog(
                onDismiss = { vm.authorPrompt = false }, title = "Your name on comments",
                confirm = "Save", onConfirm = { vm.toolPrefs.author = name.trim(); vm.authorPrompt = false; vm.prefsChanged++ },
                dismiss = "Skip",
            ) {
                Text("Add a name to show who made each comment. You can change it later in Preferences.", color = c.text, fontSize = 14.sp)
                Spacer(Modifier.height(14.dp))
                PdfField(name, { name = it }, "Your name")
            }
        }
        vm.readOnlyPrompt?.let { d ->
            PdfDialog(
                onDismiss = { vm.answerReadOnly(null) }, title = "File is read-only",
                confirm = "Save a copy", onConfirm = { vm.answerReadOnly(true) },
                dismiss = "Discard", onDismissButton = { vm.answerReadOnly(false) },
            ) { Text("Your changes to ${d.name} can't be saved where the file is. Save a copy to keep them.", color = c.text, fontSize = 14.sp) }
        }
        vm.linkPrompt?.let { url ->
            val shown = runCatching { android.net.Uri.parse(url).let { u -> "${u.scheme}://${u.host}${if (u.port > 0) ":${u.port}" else ""}" } }.getOrDefault(url)
            PdfDialog(onDismiss = { vm.answerLink(false) }, title = "Open web page", confirm = "Open", onConfirm = { vm.answerLink(true) }) {
                Text("Do you want to open the page at $shown?", color = c.text, fontSize = 14.sp)
            }
        }
        vm.confirmDeleteAllBookmarks?.let { d ->
            PdfDialog(
                onDismiss = { vm.confirmDeleteAllBookmarks = null }, title = "Delete all bookmarks?",
                confirm = "Delete all", onConfirm = { vm.confirmDeleteAllBookmarks = null; vm.deleteAllBookmarks(d) }, destructive = true,
            ) { Text("All of this document's bookmarks are removed.", color = c.text, fontSize = 14.sp) }
        }
        vm.readingModeBlocked?.let {
            PdfDialog(onDismiss = { vm.readingModeBlocked = null }, title = "Reading Mode", confirm = "OK", onConfirm = { vm.readingModeBlocked = null }, dismiss = null) {
                Text("Tools work on the page itself. Choose Continuous or Single page in View settings to use this one.", color = c.text, fontSize = 14.sp)
            }
        }
        vm.busy?.let { message ->
            PdfDialog(onDismiss = {}, title = null, confirm = null, dismiss = null) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    CircularProgressIndicator(Modifier.size(28.dp), color = c.accent, strokeWidth = 3.dp)
                    Spacer(Modifier.width(20.dp))
                    Text(message, color = c.text)
                }
            }
        }
    }
    }
}

/** "Enter password": the file's name, a masked field with an eye, OK and Cancel; wrong turns it red. */
@Composable
private fun PasswordDialog(vm: AppViewModel, req: PasswordRequest) {
    val c = pdfColors
    var pw by remember(req) { mutableStateOf("") }
    val focus = remember { FocusRequester() }
    LaunchedEffect(req) { runCatching { focus.requestFocus() } }
    PdfDialog(onDismiss = { vm.answerPassword(null) }, title = "Enter password", confirm = "OK", onConfirm = { vm.answerPassword(pw) }) {
        Text(buildAnnotatedString {
            withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { append(req.file.name) }
            append(" is protected. Please enter the password to open the document.")
        }, color = c.text, fontSize = 14.sp)
        Spacer(Modifier.height(14.dp))
        PdfField(
            pw, { pw = it }, "Password", error = req.wrong && pw.isEmpty(), password = true, focus = focus,
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { if (pw.isNotEmpty()) vm.answerPassword(pw) }),
        )
    }
}

/** The open-source licences of what the app is built from. */
@Composable
private fun LicencesScreen(vm: AppViewModel) {
    val c = pdfColors
    val context = LocalContext.current
    androidx.activity.compose.BackHandler { vm.showLicences = false }
    val inter = remember { runCatching { context.assets.open("licences/OFL-Inter.txt").bufferedReader().readText() }.getOrDefault("") }
    val lucide = remember { runCatching { context.assets.open("licences/LICENSE-lucide.txt").bufferedReader().readText() }.getOrDefault("") }
    Column(Modifier.fillMaxSize().background(c.surface).safeTopPadding().navigationBarsPadding()) {
        Row(Modifier.fillMaxWidth().height(56.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.width(56.dp), contentAlignment = Alignment.Center) { BarIcon(LucideIcons.ArrowLeft, "Back", { vm.showLicences = false }) }
            Text("Open-source licences", color = c.text, fontSize = 18.sp, fontWeight = FontWeight.SemiBold)
        }
        Column(Modifier.verticalScroll(rememberScrollState()).padding(horizontal = 20.dp)) {
            @Composable
            fun entry(name: String, licence: String) {
                Text(name, color = c.text, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(top = 14.dp))
                Text(licence, color = c.textSecondary, fontSize = 13.sp)
            }
            entry("PdfCraft (engine and app)", "MIT OR Apache-2.0, by storytold and contributors")
            entry("UniFFI", "MPL-2.0, Mozilla")
            entry("JNA", "Apache-2.0")
            entry("AndroidX, Jetpack Compose, Kotlin coroutines", "Apache-2.0")
            entry("Lucide icons", "ISC, Lucide Contributors")
            entry("Inter typeface", "SIL Open Font License 1.1, The Inter Project Authors")
            entry("Text recognition models (ocrs)", "CC-BY-SA-4.0, Robert Knight; trained on HierText")
            entry("hayro, lopdf, boa and the engine's other Rust dependencies", "MIT, Apache-2.0 and compatible licences; see NOTICE in the source")
            Spacer(Modifier.height(20.dp))
            Text(lucide, color = c.textSecondary, fontSize = 11.sp)
            Spacer(Modifier.height(20.dp))
            Text(inter, color = c.textSecondary, fontSize = 11.sp)
            Spacer(Modifier.height(32.dp))
        }
    }
}
