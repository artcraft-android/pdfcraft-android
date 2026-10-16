package io.github.artcraftandroid.pdfcraft.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.OutlinedTextFieldDefaults
import androidx.compose.material3.RadioButton
import androidx.compose.material3.RadioButtonDefaults
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.focus.focusRequester
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.widthIn
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.artcraftandroid.pdfcraft.engine.toBitmap
import io.github.artcraftandroid.pdfcraft.ui.viewer.formatSize
import kotlin.math.roundToInt
import kotlinx.coroutines.withContext

/** A tool's full screen: a top bar with back and title, the content, and the action button at the bottom. */
@Composable
fun ToolScaffold(
    title: String,
    onBack: () -> Unit,
    action: String?,
    actionEnabled: Boolean,
    onAction: () -> Unit,
    trailing: (@Composable () -> Unit)? = null,
    /** Black under the camera and the viewer's band behind the navigation buttons, as the viewer's own screens have. */
    viewerBands: Boolean = false,
    content: @Composable ColumnScope.() -> Unit,
) {
    val c = pdfColors
    BackHandler(onBack = onBack)
    Column(
        Modifier.fillMaxSize().background(if (viewerBands) Color.Black else c.surface).safeTopPadding()
            .background(if (viewerBands) c.navBand else c.surface).navigationBarsPadding().background(c.surface).imePadding(),
    ) {
        Row(
            Modifier.fillMaxWidth().height(io.github.artcraftandroid.pdfcraft.ui.viewer.TopBarHeight).then(
                if (viewerBands) Modifier.drawBehind { drawRect(c.divider, Offset(0f, size.height - 1.dp.toPx()), Size(size.width, 1.dp.toPx())) } else Modifier,
            ),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(Modifier.width(56.dp), contentAlignment = Alignment.Center) { BarIcon(LucideIcons.ArrowLeft, "Back", onBack, iconSize = 27.dp) }
            Spacer(Modifier.width(16.dp))
            Text(title, color = c.label, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
            trailing?.invoke()
        }
        Column(Modifier.weight(1f).fillMaxWidth(), content = content)
        if (action != null) {
            Button(
                onClick = onAction,
                enabled = actionEnabled,
                colors = ButtonDefaults.buttonColors(containerColor = c.button, contentColor = Color.White, disabledContainerColor = c.pressed),
                shape = RoundedCornerShape(24.dp),
                modifier = Modifier.fillMaxWidth().padding(16.dp).height(48.dp),
            ) { Text(action, fontWeight = FontWeight.SemiBold, fontSize = 16.sp) }
        }
    }
}

/** A file as the tools show it: an icon, its name and size. */
@Composable
fun FileCard(name: String, size: Long?, modifier: Modifier = Modifier, trailing: (@Composable () -> Unit)? = null) {
    val c = pdfColors
    Row(
        modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp).clip(PdfShapes.Medium).background(c.surfaceRaised).padding(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        PdfBadge()
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Text(name.substringBeforeLast('.'), color = c.text, fontSize = 15.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(listOfNotNull(name.substringAfterLast('.', "").uppercase().ifEmpty { null }, size?.let { formatSize(it) }).joinToString("  •  "), color = c.textSecondary, fontSize = 13.sp)
        }
        trailing?.invoke()
    }
}

/** The red document badge with "PDF" on it. */
@Composable
fun PdfBadge(size: Int = 36) {
    val c = pdfColors
    Box(Modifier.size(size.dp, (size * 1.2f).dp).clip(RoundedCornerShape(4.dp)).background(c.red), contentAlignment = Alignment.BottomCenter) {
        Text("PDF", color = Color.White, fontSize = (size / 3.6f).sp, fontWeight = FontWeight.Bold, modifier = Modifier.padding(bottom = 3.dp))
    }
}

/**
 * A tool page as the reference lays its own out: a bar with only back (and [trailing]), a large
 * title, the content, then the action as a blue pill at the lower right.
 */
@Composable
fun ToolPage(
    title: String,
    onBack: () -> Unit,
    action: String,
    actionEnabled: Boolean,
    onAction: () -> Unit,
    trailing: (@Composable () -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    val c = pdfColors
    BackHandler(onBack = onBack)
    // the keyboard covers the pill rather than lifting it, as the reference's does
    Column(Modifier.fillMaxSize().background(c.surface).safeTopPadding().background(c.navBand).navigationBarsPadding().background(c.surface)) {
        Row(Modifier.fillMaxWidth().height(io.github.artcraftandroid.pdfcraft.ui.viewer.TopBarHeight), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.width(56.dp), contentAlignment = Alignment.Center) { BarIcon(LucideIcons.ArrowLeft, "Back", onBack, iconSize = 27.dp) }
            Spacer(Modifier.weight(1f))
            trailing?.invoke()
            Spacer(Modifier.width(8.dp))
        }
        Text(title, color = c.label, fontSize = 20.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.padding(start = 24.dp, top = 16.dp, bottom = 14.dp))
        Column(Modifier.weight(1f).fillMaxWidth(), content = content)
        Row(Modifier.fillMaxWidth().padding(end = 24.dp, bottom = 16.dp, top = 8.dp), horizontalArrangement = Arrangement.End) {
            ActionPill(action, actionEnabled, onAction)
        }
    }
}

/** The blue pill a tool page ends with (grey while it can't be used). */
@Composable
fun ActionPill(label: String, enabled: Boolean, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val c = pdfColors
    Box(
        modifier.heightIn(min = 40.dp).widthIn(min = 90.dp).clip(RoundedCornerShape(20.dp))
            .background(if (enabled) c.button else c.pressed).clickable(enabled = enabled, role = androidx.compose.ui.semantics.Role.Button, onClick = onClick)
            .padding(horizontal = 22.dp),
        contentAlignment = Alignment.Center,
    ) { Text(label, color = if (enabled) Color.White else c.textSecondary, fontSize = 16.sp, fontWeight = FontWeight.SemiBold) }
}

/** The file a tool works on, in an outlined card: a picture of its first page, its name, type and size. */
@Composable
fun OutlinedFileCard(vm: AppViewModel, doc: OpenDocument) {
    val c = pdfColors
    var thumb by remember(doc.info.generation) { mutableStateOf<ImageBitmap?>(null) }
    LaunchedEffect(doc.info.generation) {
        val eng = vm.engine ?: return@LaunchedEffect
        val p = doc.info.pages.firstOrNull() ?: return@LaunchedEffect
        val scale = 120f / maxOf(p.width, p.height).coerceAtLeast(1f)
        thumb = runCatching { withContext(eng.thumbnails) { eng.session.renderPage(doc.id, 0u, scale, null).toBitmap().asImageBitmap() } }.getOrNull()
    }
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 24.dp).clip(PdfShapes.Small).border(1.dp, c.divider, PdfShapes.Small).padding(horizontal = 12.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(34.dp, 40.dp).background(Color.White)) {
            thumb?.let { Image(it, null, modifier = Modifier.fillMaxSize(), contentScale = ContentScale.Crop) }
        }
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Text(doc.name.substringBeforeLast('.'), color = c.label, fontSize = 16.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text("PDF • ${formatSize(doc.info.fileSize.toLong())}", color = c.text, fontSize = 13.sp)
        }
    }
}

/** A radio choice of a tool page: the circle, a bold name and a line under it. */
@Composable
private fun ToolChoice(selected: Boolean, name: String, description: String?, onClick: () -> Unit) {
    val c = pdfColors
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(start = 8.dp, end = 16.dp, top = if (description == null) 0.dp else 6.dp, bottom = if (description == null) 0.dp else 8.dp),
        verticalAlignment = if (description == null) Alignment.CenterVertically else Alignment.Top,
    ) {
        RadioButton(selected = selected, onClick = onClick, colors = RadioButtonDefaults.colors(selectedColor = c.toolAccent, unselectedColor = c.icon))
        Column(Modifier.padding(start = 6.dp, top = if (description == null) 0.dp else 10.dp)) {
            Text(name, color = c.label, fontSize = 16.sp, fontWeight = if (description == null) FontWeight.Normal else FontWeight.SemiBold)
            if (description != null) {
                Spacer(Modifier.height(6.dp))
                Text(description, color = c.text, fontSize = 14.sp)
            }
        }
    }
}

/** Compress PDF: one of three levels (medium by default), then a smaller copy in Downloads. */
@Composable
fun CompressScreen(vm: AppViewModel, doc: OpenDocument) {
    val c = pdfColors
    var level by remember { mutableStateOf(1) }
    val levels = listOf(
        "High compression" to "Smallest file, images softer",
        "Medium compression" to "Balanced file size and quality",
        "Low compression" to "Bigger file, images sharpest",
    )
    ToolPage("Compress PDF", { vm.toolScreen = null }, "Compress", true, { vm.compress(doc, level) }) {
        OutlinedFileCard(vm, doc)
        Text("Compression level:", color = c.label, fontSize = 15.sp, modifier = Modifier.padding(start = 24.dp, top = 28.dp, bottom = 4.dp))
        levels.forEachIndexed { i, (name, desc) -> ToolChoice(level == i, name, desc) { level = i } }
    }
}

/** A password field of Set password: its label above an outlined box, with the eye to show it. */
@Composable
private fun PasswordField(label: String, value: String, onValue: (String) -> Unit, visible: Boolean, onToggle: (() -> Unit)?, error: Boolean, focus: androidx.compose.ui.focus.FocusRequester? = null) {
    val c = pdfColors
    var focused by remember { mutableStateOf(false) }
    Text(label, color = c.label, fontSize = 16.sp, modifier = Modifier.padding(start = 24.dp, top = 20.dp, bottom = 8.dp))
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 24.dp).height(44.dp).clip(PdfShapes.Small).background(c.menu)
            .border(if (focused) 2.dp else 1.dp, if (error) c.red else if (focused) c.toolAccent else c.divider, PdfShapes.Small).padding(start = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        androidx.compose.foundation.text.BasicTextField(
            value = value, onValueChange = onValue, singleLine = true,
            textStyle = androidx.compose.ui.text.TextStyle(color = c.text, fontSize = 17.sp),
            cursorBrush = androidx.compose.ui.graphics.SolidColor(c.toolAccent),
            visualTransformation = if (visible) VisualTransformation.None else PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
            modifier = Modifier.weight(1f).then(if (focus != null) Modifier.focusRequester(focus) else Modifier)
                .onFocusChanged { focused = it.isFocused }.semantics { contentDescription = label },
        )
        BarIcon(if (visible) LucideIcons.Eye else LucideIcons.EyeOff, if (visible) "Hide password" else "Show password", { onToggle?.invoke() }, size = 44.dp)
    }
}

/** Set password: a password twice (6 to 128 printable characters), then a protected copy in Downloads. */
@Composable
fun ProtectScreen(vm: AppViewModel, doc: OpenDocument) {
    val c = pdfColors
    var pw by remember { mutableStateOf("") }
    var confirm by remember { mutableStateOf("") }
    var visible by remember { mutableStateOf(false) }
    var info by remember { mutableStateOf(false) }
    val focus = remember { androidx.compose.ui.focus.FocusRequester() }
    LaunchedEffect(Unit) { runCatching { focus.requestFocus() } }
    val bad = pw.isNotEmpty() && (pw.length !in 6..128 || pw.any { it < '!' || it > '~' })
    val mismatch = confirm.isNotEmpty() && confirm != pw
    ToolPage("Set password", { vm.toolScreen = null }, "Set password", pw.isNotEmpty() && confirm.isNotEmpty() && !bad && !mismatch, { vm.protect(doc, pw) },
        trailing = { BarIcon(LucideIcons.Info, "About passwords", { info = true }) }) {
        Column(Modifier.verticalScroll(rememberScrollState())) {
            Text("A protected copy of this file is saved.", color = c.label, fontSize = 16.sp, modifier = Modifier.padding(horizontal = 24.dp))
            PasswordField("New password *", pw, { pw = it }, visible, { visible = !visible }, bad, focus)
            if (bad) Text("Use 6 to 128 characters: letters, numbers and symbols, no spaces.", color = c.red, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 24.dp, vertical = 4.dp))
            PasswordField("Repeat the password *", confirm, { confirm = it }, visible, { visible = !visible }, mismatch)
            if (mismatch) Text("The passwords don't match.", color = c.red, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 24.dp, vertical = 4.dp))
        }
    }
    if (info) {
        PdfDialog(onDismiss = { info = false }, title = "About passwords", confirm = "OK", onConfirm = { info = false }, dismiss = null) {
            Text("The copy is encrypted with AES-256. Keep the password safe: it can't be recovered. To remove the password later, open the copy with it and choose Remove password.", color = c.text, fontSize = 14.sp)
        }
    }
}

/** Export PDF: Document (Word, RTF, Excel, PowerPoint) or Image (JPEG, PNG) of its pages. */
@Composable
fun ExportScreen(vm: AppViewModel, doc: OpenDocument) {
    val c = pdfColors
    var tab by remember { mutableStateOf(0) }
    // the image format used last time comes first chosen
    var format by remember(tab) { mutableStateOf(if (tab == 1 && vm.lastImageFormat == "png") 1 else 0) }
    val formats = if (tab == 0) {
        listOf("Word (docx)" to "docx", "Rich Text Format (rtf)" to "rtf", "Excel (xlsx)" to "xlsx", "PowerPoint (pptx)" to "pptx")
    } else {
        listOf("JPEG (jpeg)" to "jpg", "PNG (png)" to "png")
    }
    ToolPage("Export PDF", { vm.toolScreen = null }, "Export", true, {
        val ext = formats[format].second
        if (tab == 0) vm.exportDocument(doc, ext) else vm.exportImages(doc, ext)
    }) {
        // the tabs sit at the left, each underlined across its own label
        Box(Modifier.fillMaxWidth().padding(horizontal = 8.dp)) {
            Box(Modifier.align(Alignment.BottomStart).fillMaxWidth().height(1.dp).background(c.divider))
            Row {
                listOf("Document", "Image").forEachIndexed { i, label ->
                    Column(Modifier.clickable { tab = i }.padding(horizontal = 8.dp).width(androidx.compose.foundation.layout.IntrinsicSize.Max)) {
                        Text(label, color = if (tab == i) c.label else c.text, fontSize = 16.sp, modifier = Modifier.padding(vertical = 12.dp))
                        Box(Modifier.fillMaxWidth().height(2.dp).background(if (tab == i) c.label else Color.Transparent))
                    }
                    if (i == 0) Spacer(Modifier.width(14.dp))
                }
            }
        }
        Spacer(Modifier.height(12.dp))
        if (tab == 0 && doc.info.encrypted && !doc.info.allowsCopying) {
            Text("This document's security settings don't allow exporting its text. Export images of its pages instead.", color = c.red, fontSize = 14.sp, modifier = Modifier.padding(20.dp))
        }
        formats.forEachIndexed { i, (label, _) -> ToolChoice(format == i, label, null) { format = i } }
    }
}

/**
 * Create PDF's first page: where the files come from (the photos, or any file through the
 * system's picker), and a scan to make a new one. Choosing closes this page.
 */
@Composable
fun CreateSourcesScreen(vm: AppViewModel) {
    val c = pdfColors
    val back = { vm.toolScreen = null }
    BackHandler(onBack = back)
    Column(Modifier.fillMaxSize().background(c.surface).safeTopPadding().background(c.navBand).navigationBarsPadding().background(c.surface)) {
        Row(Modifier.fillMaxWidth().height(io.github.artcraftandroid.pdfcraft.ui.viewer.TopBarHeight), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.width(56.dp), contentAlignment = Alignment.Center) { BarIcon(LucideIcons.ArrowLeft, "Back", back, iconSize = 27.dp) }
            Spacer(Modifier.width(4.dp))
            Text("Choose files to make a PDF", color = c.label, fontSize = 17.sp, fontWeight = FontWeight.SemiBold)
        }
        Spacer(Modifier.height(8.dp))
        SourceRow(LucideIcons.Images, "Photos") { vm.toolScreen = null; vm.pickPhotosToPdf() }
        SourceRow(LucideIcons.FolderOpen, "Browse files") { vm.toolScreen = null; vm.pickFiles(FilePickKind.Create) }
        Text("Or make a new one", color = c.textSecondary, fontSize = 14.sp, modifier = Modifier.padding(start = 20.dp, top = 20.dp, bottom = 6.dp))
        Box(Modifier.padding(horizontal = 16.dp).fillMaxWidth().height(1.dp).background(c.divider))
        SourceRow(LucideIcons.Camera, "Scan a document") { vm.toolScreen = null; vm.startScan() }
    }
}

@Composable
private fun SourceRow(icon: androidx.compose.ui.graphics.vector.ImageVector, label: String, onClick: () -> Unit) {
    val c = pdfColors
    Row(Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(40.dp).clip(PdfShapes.Small).background(c.pressed), contentAlignment = Alignment.Center) {
            Icon(icon, null, tint = c.icon, modifier = Modifier.size(22.dp))
        }
        Spacer(Modifier.width(16.dp))
        Text(label, color = c.label, fontSize = 16.sp)
    }
}

/** The tool screen that is open. */
@Composable
fun ToolScreenHost(vm: AppViewModel, screen: ToolScreen) {
    when (screen) {
        is ToolScreen.Compress -> CompressScreen(vm, screen.doc)
        is ToolScreen.Combine -> CombineScreen(vm)
        is ToolScreen.Protect -> ProtectScreen(vm, screen.doc)
        is ToolScreen.Crop -> CropScreen(vm, screen.doc)
        is ToolScreen.Export -> ExportScreen(vm, screen.doc)
        ToolScreen.Scan -> ScanScreen(vm)
        ToolScreen.Create -> CreateSourcesScreen(vm)
    }
}


/**
 * Combine files, as the reference's: "Combine files" with an outlined Add files at the top, the
 * files in order (the open document first when there is one; hold the grip to reorder, the minus to
 * remove), and Combine across the bottom once there are two to twelve of them.
 */
@Composable
fun CombineScreen(vm: AppViewModel) {
    var naming by remember { mutableStateOf(false) }
    val c = pdfColors
    val files = vm.combineFiles
    var dragIndex by remember { mutableStateOf<Int?>(null) }
    var dragOffset by remember { mutableFloatStateOf(0f) }
    val rowPx = with(LocalDensity.current) { 58.dp.toPx() }
    val ok = files.size in 2..12
    val back = { vm.toolScreen = null; vm.combineFiles.clear() }
    BackHandler(onBack = back)
    Column(Modifier.fillMaxSize().background(c.surface).safeTopPadding().background(c.navBand).navigationBarsPadding().background(c.surface)) {
        Row(
            Modifier.fillMaxWidth().height(io.github.artcraftandroid.pdfcraft.ui.viewer.TopBarHeight)
                .drawBehind { drawRect(c.divider, Offset(0f, size.height - 1.dp.toPx()), Size(size.width, 1.dp.toPx())) },
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(Modifier.width(56.dp), contentAlignment = Alignment.Center) { BarIcon(LucideIcons.ArrowLeft, "Back", back, iconSize = 27.dp) }
            Spacer(Modifier.width(4.dp))
            Text(
                if (files.size > 1) "${files.size} files" else "Combine files",
                color = c.label, fontSize = 17.sp, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f),
            )
            Box(
                Modifier.padding(end = 20.dp).height(36.dp).clip(RoundedCornerShape(18.dp)).border(2.dp, c.label, RoundedCornerShape(18.dp))
                    .clickable { vm.pickFiles(FilePickKind.Combine) }.padding(horizontal = 16.dp),
                contentAlignment = Alignment.Center,
            ) { Text("Add files", color = c.label, fontSize = 16.sp, fontWeight = FontWeight.SemiBold) }
        }
        if (files.size > 12) Text("Remove some files: up to 12 can be combined.", color = c.red, fontSize = 13.sp, modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp))
        LazyColumn(Modifier.fillMaxWidth().weight(1f).padding(horizontal = 8.dp)) {
            itemsIndexed(files, key = { _, f -> f.uri.toString() }) { i, f ->
                Row(
                    Modifier.fillMaxWidth().height(58.dp)
                        .offset { IntOffset(0, if (dragIndex == i) dragOffset.roundToInt() else 0) }
                        .background(if (dragIndex == i) c.pressed else Color.Transparent)
                        .drawBehind { drawRect(c.divider, Offset(8.dp.toPx(), size.height - 1.dp.toPx()), Size(size.width - 16.dp.toPx(), 1.dp.toPx())) },
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(
                        Modifier.size(44.dp).pointerInput(f.uri) {
                            detectDragGestures(
                                onDragStart = { dragIndex = files.indexOf(f); dragOffset = 0f },
                                onDragEnd = { dragIndex = null; dragOffset = 0f },
                                onDragCancel = { dragIndex = null; dragOffset = 0f },
                            ) { change, d ->
                                change.consume()
                                val from = dragIndex ?: return@detectDragGestures
                                dragOffset += d.y
                                val steps = (dragOffset / rowPx).toInt()
                                val to = (from + steps).coerceIn(0, files.lastIndex)
                                if (to != from) {
                                    files.add(to, files.removeAt(from))
                                    dragIndex = to
                                    dragOffset -= (to - from) * rowPx
                                }
                            }
                        }.semantics { contentDescription = "Drag to reorder" },
                        contentAlignment = Alignment.Center,
                    ) { Icon(LucideIcons.GripVertical, null, tint = c.icon, modifier = Modifier.size(20.dp)) }
                    Column(Modifier.weight(1f)) {
                        Text(f.name.substringBeforeLast('.'), color = c.label, fontSize = 17.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(listOfNotNull(f.name.substringAfterLast('.', "").uppercase(), f.size?.let { formatSize(it) }).joinToString(" • "), color = c.text, fontSize = 14.sp)
                    }
                    BarIcon(LucideIcons.CircleMinus, "Remove ${f.name}", { files.remove(f); vm.tell("File removed") })
                }
            }
        }
        Box(
            Modifier.fillMaxWidth().padding(horizontal = 10.dp, vertical = 12.dp).height(42.dp).clip(RoundedCornerShape(21.dp))
                .background(if (ok) c.button else c.pressed).clickable(enabled = ok) { naming = true },
            contentAlignment = Alignment.Center,
        ) { Text("Combine as PDF", color = if (ok) Color.White else c.textSecondary, fontSize = 17.sp, fontWeight = FontWeight.SemiBold) }
        if (naming) {
            var name by remember { mutableStateOf("Combine " + java.text.DateFormat.getDateInstance(java.text.DateFormat.LONG).format(java.util.Date())) }
            val focus = remember { androidx.compose.ui.focus.FocusRequester() }
            PdfDialog(
                onDismiss = { naming = false }, title = "Name the combined file", confirm = "Save",
                onConfirm = { naming = false; vm.combine(name.trim().ifEmpty { "Combine" }) },
                confirmEnabled = name.isNotBlank(),
            ) { PdfField(name, { name = it }, "File name", focus = focus) }
        }
    }
}


/** A file picked for a tool: where it is, its name and size. */
data class PickedFile(val uri: android.net.Uri, val name: String, val size: Long?)

/** What a file picker was opened for. */
enum class FilePickKind { Combine, Create, Insert, Open }


/**
 * Crop pages, laid out as the reference's: the page inset 32 dp on black, a crop frame with ringed
 * corner handles and pill handles mid-edge, the page number in a grey pill under it, then the All
 * pages switch and Reset. Save and Reset stay grey until the frame moves.
 */
@Composable
fun CropScreen(vm: AppViewModel, doc: OpenDocument) {
    val c = pdfColors
    var page by remember { mutableStateOf(doc.viewer.currentPage) }
    var allPages by remember { mutableStateOf(false) }
    // the crop as fractions of the page: left, top, right, bottom
    var rect by remember { mutableStateOf(floatArrayOf(0f, 0f, 1f, 1f)) }
    var bitmap by remember(page) { mutableStateOf<ImageBitmap?>(null) }
    val p = doc.info.pages.getOrNull(page)
    LaunchedEffect(page) {
        val eng = vm.engine ?: return@LaunchedEffect
        val info = p ?: return@LaunchedEffect
        val scale = 1200f / maxOf(info.width, info.height).coerceAtLeast(1f)
        bitmap = runCatching { withContext(eng.thumbnails) { eng.session.renderPage(doc.id, page.toUInt(), scale, null).toBitmap().asImageBitmap() } }.getOrNull()
    }
    val changed = !(rect[0] == 0f && rect[1] == 0f && rect[2] == 1f && rect[3] == 1f)
    ToolScaffold(if (allPages) "All pages" else "Current page", { vm.toolScreen = null }, null, false, {},
        viewerBands = true,
        trailing = {
            Text(
                "Save", color = if (changed) c.label else c.textSecondary.copy(alpha = 0.5f), fontSize = 17.sp,
                modifier = Modifier.clickable(enabled = changed) {
                    val info = p ?: return@clickable
                    vm.crop(doc, if (allPages) null else page, floatArrayOf(rect[0] * info.width, rect[1] * info.height, rect[2] * info.width, rect[3] * info.height))
                }.padding(horizontal = 16.dp, vertical = 12.dp),
            )
        }) {
        BoxWithConstraints(Modifier.weight(1f).fillMaxWidth().background(c.desk), contentAlignment = Alignment.Center) {
            val info = p ?: return@BoxWithConstraints
            val ratio = info.width / info.height.coerceAtLeast(1f)
            // the page sits 32 dp in from the sides; touches are taken a margin beyond its edges
            val margin = 32f
            val boxW = minOf(maxWidth.value - 2 * margin, (maxHeight.value - 2 * margin) * ratio).coerceAtLeast(1f)
            val boxH = boxW / ratio
            val density = LocalDensity.current
            val wPx = with(density) { boxW.dp.toPx() }
            val hPx = with(density) { boxH.dp.toPx() }
            val mPx = with(density) { margin.dp.toPx() }
            Box(
                Modifier.size((boxW + 2 * margin).dp, (boxH + 2 * margin).dp).pointerInput(page) {
                    var mode = -1
                    detectDragGestures(
                        onDragStart = { o ->
                            val x = (o.x - mPx) / wPx
                            val y = (o.y - mPx) / hPx
                            val near = 0.08f
                            val l = kotlin.math.abs(x - rect[0]) < near
                            val r = kotlin.math.abs(x - rect[2]) < near
                            val t = kotlin.math.abs(y - rect[1]) < near
                            val b = kotlin.math.abs(y - rect[3]) < near
                            mode = when {
                                l && t -> 0; r && t -> 1; r && b -> 2; l && b -> 3
                                l -> 4; t -> 5; r -> 6; b -> 7
                                x in rect[0]..rect[2] && y in rect[1]..rect[3] -> 8
                                else -> -1
                            }
                        },
                    ) { change, d ->
                        change.consume()
                        val dx = d.x / wPx
                        val dy = d.y / hPx
                        val n = rect.copyOf()
                        val min = 0.1f
                        when (mode) {
                            0 -> { n[0] += dx; n[1] += dy }
                            1 -> { n[2] += dx; n[1] += dy }
                            2 -> { n[2] += dx; n[3] += dy }
                            3 -> { n[0] += dx; n[3] += dy }
                            4 -> n[0] += dx
                            5 -> n[1] += dy
                            6 -> n[2] += dx
                            7 -> n[3] += dy
                            8 -> {
                                val mx = dx.coerceIn(-n[0], 1f - n[2])
                                val my = dy.coerceIn(-n[1], 1f - n[3])
                                n[0] += mx; n[2] += mx; n[1] += my; n[3] += my
                            }
                            else -> return@detectDragGestures
                        }
                        n[0] = n[0].coerceIn(0f, n[2] - min); n[1] = n[1].coerceIn(0f, n[3] - min)
                        n[2] = n[2].coerceIn(n[0] + min, 1f); n[3] = n[3].coerceIn(n[1] + min, 1f)
                        rect = n
                    }
                },
            ) {
                Box(Modifier.padding(margin.dp).fillMaxSize().background(Color.White)) {
                    bitmap?.let { Image(it, null, modifier = Modifier.fillMaxSize(), contentScale = ContentScale.FillBounds) }
                }
                val ring = c.toolAccent
                val fill = c.menu
                androidx.compose.foundation.Canvas(Modifier.fillMaxSize()) {
                    val l = mPx + rect[0] * wPx
                    val t = mPx + rect[1] * hPx
                    val r = mPx + rect[2] * wPx
                    val b = mPx + rect[3] * hPx
                    val dim = Color.Black.copy(alpha = 0.45f)
                    drawRect(dim, Offset(mPx, mPx), Size(wPx, t - mPx))
                    drawRect(dim, Offset(mPx, b), Size(wPx, mPx + hPx - b))
                    drawRect(dim, Offset(mPx, t), Size(l - mPx, b - t))
                    drawRect(dim, Offset(r, t), Size(mPx + wPx - r, b - t))
                    val line = 2.dp.toPx()
                    drawRect(ring, Offset(l, t), Size(r - l, b - t), style = Stroke(line))
                    // corners: 18 dp rings; edges: 44 x 18 dp pills along the edge
                    val corner = 9.dp.toPx()
                    for ((hx, hy) in listOf(l to t, r to t, r to b, l to b)) {
                        drawCircle(fill, corner, Offset(hx, hy))
                        drawCircle(ring, corner - line / 2, Offset(hx, hy), style = Stroke(line))
                    }
                    val long = 44.dp.toPx()
                    val short = 18.dp.toPx()
                    for ((hx, hy, across) in listOf(Triple((l + r) / 2, t, true), Triple((l + r) / 2, b, true), Triple(l, (t + b) / 2, false), Triple(r, (t + b) / 2, false))) {
                        val w = if (across) long else short
                        val h = if (across) short else long
                        val tl = Offset(hx - w / 2, hy - h / 2)
                        val cr = androidx.compose.ui.geometry.CornerRadius(short / 2)
                        drawRoundRect(fill, tl, Size(w, h), cr)
                        drawRoundRect(ring, Offset(tl.x + line / 2, tl.y + line / 2), Size(w - line, h - line), cr, style = Stroke(line))
                    }
                }
            }
        }
        // the page in a grey pill (with arrows to step through the pages when cropping them all)
        Row(Modifier.fillMaxWidth().background(c.desk).height(88.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.Center) {
            if (allPages) BarIcon(LucideIcons.ChevronLeft, "Previous page", { page = (page - 1).coerceAtLeast(0) }, enabled = page > 0)
            Box(
                Modifier.widthIn(min = 102.dp).height(32.dp).clip(RoundedCornerShape(16.dp)).background(Color(0xFFD1D1D1)).padding(horizontal = 16.dp),
                contentAlignment = Alignment.Center,
            ) { Text("Page ${page + 1}", color = Color(0xFF2C2C2C), fontSize = 15.sp) }
            if (allPages) BarIcon(LucideIcons.ChevronRight, "Next page", { page = (page + 1).coerceAtMost(doc.pageCount - 1) }, enabled = page < doc.pageCount - 1)
        }
        Row(Modifier.fillMaxWidth().height(54.dp).background(c.surface).padding(start = 12.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            PdfSwitch(allPages, { allPages = it })
            Spacer(Modifier.width(16.dp))
            Text("All pages", color = c.label, fontSize = 16.sp, modifier = Modifier.weight(1f))
            Text(
                "Reset", color = if (changed) c.label else c.textSecondary.copy(alpha = 0.5f), fontSize = 16.sp,
                modifier = Modifier.clickable(enabled = changed) { rect = floatArrayOf(0f, 0f, 1f, 1f) }.padding(horizontal = 12.dp, vertical = 12.dp),
            )
        }
    }
}


/** A scanned page: the photo's file and its thumbnail. */
data class ScanPage(val file: java.io.File, val thumb: ImageBitmap)

/** The looks a scan can have, and how the engine's camera clean-up gives each. */
enum class ScanFilter(val label: String, val mode: String, val level: Int) {
    Original("Original", "original", 0),
    AutoColor("Auto colour", "colour", 50),
    Grayscale("Grayscale", "greyscale", 40),
    Whiteboard("Whiteboard", "colour", 85),
    LightText("Light text", "bw", 60),
}

/** Scan: pages taken with the camera, reviewed (retake, delete, look), then saved as a PDF. */
@Composable
fun ScanScreen(vm: AppViewModel) {
    val c = pdfColors
    val pages = vm.scanPages
    var filter by remember { mutableStateOf(ScanFilter.AutoColor) }
    var current by remember { mutableStateOf(0) }
    LaunchedEffect(Unit) { if (pages.isEmpty()) vm.takeScanPhoto() }
    ToolScaffold(
        if (pages.isEmpty()) "Scan" else "Page ${current.coerceAtMost(pages.lastIndex) + 1} of ${pages.size}",
        { vm.discardScan() },
        "Save PDF", pages.isNotEmpty(), { vm.saveScan(filter) },
    ) {
        Box(Modifier.weight(1f).fillMaxWidth().background(c.desk), contentAlignment = Alignment.Center) {
            val page = pages.getOrNull(current.coerceAtMost(pages.lastIndex))
            if (page != null) {
                Image(page.thumb, null, modifier = Modifier.fillMaxSize().padding(16.dp), contentScale = ContentScale.Fit)
            } else {
                Text("Take a photo of each page", color = c.textSecondary)
            }
        }
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            pages.forEachIndexed { i, p ->
                Image(p.thumb, "Page ${i + 1}", modifier = Modifier.size(56.dp, 72.dp).clip(PdfShapes.Thumb).border(2.dp, if (i == current) c.accent else Color.Transparent, PdfShapes.Thumb).clickable { current = i }, contentScale = ContentScale.Crop)
            }
            Box(Modifier.size(56.dp, 72.dp).clip(PdfShapes.Thumb).background(c.surfaceRaised).clickable { vm.takeScanPhoto() }.semantics { contentDescription = "Add page" }, contentAlignment = Alignment.Center) {
                Icon(LucideIcons.Plus, null, tint = c.icon)
            }
        }
        Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 8.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            ScanFilter.entries.forEach { f ->
                Text(
                    f.label, color = if (f == filter) c.accent else c.text, fontSize = 14.sp,
                    modifier = Modifier.clip(CircleShape).background(if (f == filter) c.pressed else Color.Transparent).clickable { filter = f }.padding(horizontal = 14.dp, vertical = 8.dp),
                )
            }
        }
        Row(Modifier.fillMaxWidth().padding(8.dp), horizontalArrangement = Arrangement.SpaceEvenly) {
            TextButton(onClick = { vm.retakeScan(current) }, enabled = pages.isNotEmpty()) { Text("Retake", color = c.accent) }
            TextButton(onClick = { if (pages.isNotEmpty()) { pages.removeAt(current.coerceAtMost(pages.lastIndex)); current = (current - 1).coerceAtLeast(0) } }, enabled = pages.isNotEmpty()) { Text("Delete", color = c.accent) }
        }
    }
}
