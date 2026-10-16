package io.github.artcraftandroid.pdfcraft.ui.viewer

import io.github.artcraftandroid.pdfcraft.ui.floatingSurface
import io.github.artcraftandroid.pdfcraft.ui.GLASS_PANEL_ALPHA
import io.github.artcraftandroid.pdfcraft.ui.FloatEdge
import io.github.artcraftandroid.pdfcraft.ui.PdfShapes
import io.github.artcraftandroid.pdfcraft.ui.safeTopPadding
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Switch
import androidx.compose.material3.SwitchDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.draw.drawBehind
import androidx.compose.foundation.layout.widthIn
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.artcraftandroid.pdfcraft.ui.AppViewModel
import io.github.artcraftandroid.pdfcraft.ui.BarIcon
import io.github.artcraftandroid.pdfcraft.ui.LucideIcons
import io.github.artcraftandroid.pdfcraft.ui.OpenDocument
import io.github.artcraftandroid.pdfcraft.ui.PdfSheet
import io.github.artcraftandroid.pdfcraft.ui.SheetDivider
import io.github.artcraftandroid.pdfcraft.ui.SheetRow
import io.github.artcraftandroid.pdfcraft.ui.SheetTitle
import io.github.artcraftandroid.pdfcraft.ui.pdfColors
import java.io.File

/** What a tap places in Fill & Sign. */
enum class FillElement(val engineType: String, val label: String) {
    Text("text", "Text"),
    Cross("cross", "Cross"),
    Check("check", "Checkmark"),
    Dot("dot", "Dot"),
    Box("box", "Rounded box"),
    Line("line", "Line"),
    Signature("signature", "Signature"),
    Initials("initials", "Initials"),
}

/** The saved signature and initials (PNG with transparency), kept in the app's private files. */
class Signatures(dir: File) {
    private val root = File(dir, "signatures").apply { mkdirs() }
    val signature = File(root, "signature.png")
    val initials = File(root, "initials.png")

    fun fileFor(initials: Boolean) = if (initials) this.initials else signature
}

/** Fill & Sign's touch layer: a tap places the chosen element; text asks for its words first. */
@Composable
fun FillSignLayer(vm: AppViewModel, doc: OpenDocument) {
    Box(
        Modifier.fillMaxSize().toolGestures(
            doc,
            onDown = {},
            onMove = {},
            onUp = { p, moved ->
                if (moved) return@toolGestures
                val at = doc.viewer.screenToPage(p) ?: return@toolGestures
                // a tap on something already placed selects it
                val placed = doc.comments.lastOrNull { c ->
                    c.fillSign && c.page.toInt() == at.page && c.rect.size == 4 &&
                        at.x in (c.rect[0] - 6)..(c.rect[2] + 6) && at.y in (c.rect[1] - 6)..(c.rect[3] + 6)
                }
                if (placed != null) {
                    vm.selectedFill = placed
                    return@toolGestures
                }
                if (vm.selectedFill != null) {
                    vm.selectedFill = null
                    return@toolGestures
                }
                // read now: the gesture handler outlives the composition it was made in
                when (val element = vm.fillElement) {
                    FillElement.Text -> vm.fillTextPrompt = TextPrompt(doc, at)
                    FillElement.Signature, FillElement.Initials -> {
                        val initials = element == FillElement.Initials
                        if (vm.signatures.fileFor(initials).isFile) vm.placeFill(doc, element, at)
                        else vm.signaturePad = SignaturePad(initials, doc, at)
                    }
                    else -> vm.placeFill(doc, element, at)
                }
            },
            onCancel = {},
        ),
    )
}

/** Fill & Sign's bar: the elements, then Sign (its own panel). On a form with fields, only Sign. */
@Composable
fun FillSignToolbar(vm: AppViewModel, doc: OpenDocument) {
    val sel = vm.fillElement
    SubToolbar {
        if (doc.info.fields == 0u) {
            SubTextField("Add text", sel == FillElement.Text) { vm.fillElement = FillElement.Text }
            SubTool(LucideIcons.X, "Add a cross", sel == FillElement.Cross, { vm.fillElement = FillElement.Cross })
            SubTool(LucideIcons.Check, "Add a checkmark", sel == FillElement.Check, { vm.fillElement = FillElement.Check })
            SubDot("Add a dot", sel == FillElement.Dot) { vm.fillElement = FillElement.Dot }
            SubTool(LucideIcons.RectangleHorizontal, "Add a rounded box", sel == FillElement.Box, { vm.fillElement = FillElement.Box })
            SubTool(LucideIcons.Minus, "Add a line", sel == FillElement.Line, { vm.fillElement = FillElement.Line })
        }
        SubTool(LucideIcons.Signature, "Sign or add initials", sel == FillElement.Signature || sel == FillElement.Initials || doc.controller.signPanel, { doc.controller.signPanel = true })
    }
}

/** Sign: the saved signature and initials (tap to place one), or making them. */
@Composable
fun SignPanel(vm: AppViewModel, doc: OpenDocument, onDismiss: () -> Unit) {
    // read so the panel redraws when a saved signature is cleared or made
    val version = vm.signaturesChanged
    // over the Fill & Sign bar, which stays in view: two dashed boxes, Add signature and Add initials
    PdfSheet(onDismiss = onDismiss, modifier = Modifier.padding(bottom = BottomBarHeight + FloatEdge + BubbleInsetBottom + 6.dp).navigationBarsPadding(), bottomBand = false) {
        Spacer(Modifier.height(7.dp))
        for (initials in listOf(false, true)) {
            val file = vm.signatures.fileFor(initials)
            val label = if (initials) "initials" else "signature"
            if (!initials) Unit else Spacer(Modifier.height(18.dp))
            SignBox(
                onClick = {
                    if (file.isFile) {
                        vm.fillElement = if (initials) FillElement.Initials else FillElement.Signature
                        onDismiss()
                        vm.tell("Tap where you want to place your $label")
                    } else {
                        onDismiss()
                        vm.signaturePad = SignaturePad(initials, doc, null)
                    }
                },
            ) {
                if (file.isFile) {
                    val bmp = remember(file.lastModified(), version) { BitmapFactory.decodeFile(file.absolutePath) }
                    if (bmp != null) Image(bmp.asImageBitmap(), "Your $label", modifier = Modifier.weight(1f).height(44.dp).padding(start = 12.dp), alignment = Alignment.CenterStart)
                    BarIcon(LucideIcons.Trash2, "Clear saved $label", { file.delete(); vm.signaturesChanged++ }, size = 40.dp)
                } else {
                    Text(if (initials) "Add initials" else "Add signature", color = pdfColors.text, fontSize = 16.sp, modifier = Modifier.weight(1f).padding(start = 22.dp))
                    Icon(LucideIcons.Plus, null, tint = pdfColors.icon, modifier = Modifier.padding(end = 9.dp).size(24.dp))
                }
            }
        }
        Spacer(Modifier.height(30.dp))
    }
}

/** A dashed 360 x 60 dp box of the signature panel. */
@Composable
private fun SignBox(onClick: () -> Unit, content: @Composable androidx.compose.foundation.layout.RowScope.() -> Unit) {
    val line = pdfColors.divider
    Box(Modifier.fillMaxWidth().padding(horizontal = 16.dp), contentAlignment = Alignment.Center) {
        Row(
            Modifier.widthIn(max = 360.dp).fillMaxWidth().height(60.dp)
                .drawBehind {
                    val w = 1.dp.toPx()
                    val dash = androidx.compose.ui.graphics.PathEffect.dashPathEffect(floatArrayOf(3.dp.toPx(), 2.dp.toPx()))
                    drawRoundRect(
                        line, topLeft = Offset(w / 2, w / 2), size = androidx.compose.ui.geometry.Size(size.width - w, size.height - w),
                        cornerRadius = androidx.compose.ui.geometry.CornerRadius(2.dp.toPx()), style = Stroke(w, pathEffect = dash),
                    )
                }
                .clickable(role = Role.Button, onClick = onClick),
            verticalAlignment = Alignment.CenterVertically,
            content = content,
        )
    }
}

/** A signature or initials being made: for [doc] at [at] (placed once done), or just saved. */
class SignaturePad(val initials: Boolean, val doc: OpenDocument?, val at: PagePoint?)

/**
 * Making a signature or initials: draw it with a finger (Clear starts over) or take it from an
 * image, then Done saves it (Save to device) and places it if a spot was tapped.
 */
@Composable
fun SignaturePadScreen(vm: AppViewModel, pad: SignaturePad, onPickImage: () -> Unit) {
    val c = pdfColors
    val strokes = remember { mutableStateListOf<MutableList<Offset>>() }
    var size by remember { mutableStateOf(IntSize.Zero) }
    var save by remember { mutableStateOf(true) }
    val ink = Color(vm.toolPrefs.fillColor)
    val imported = vm.importedSignature
    Column(Modifier.fillMaxSize().background(c.surface).safeTopPadding().navigationBarsPadding()) {
        Row(Modifier.fillMaxWidth().height(56.dp).padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            TextButton(onClick = { vm.signaturePad = null; vm.importedSignature = null }) { Text("Cancel", color = c.accent) }
            Text(if (pad.initials) "Create initials" else "Create signature", color = c.text, fontWeight = FontWeight.SemiBold, fontSize = 17.sp, modifier = Modifier.weight(1f), textAlign = androidx.compose.ui.text.style.TextAlign.Center)
            TextButton(
                onClick = {
                    val bmp = imported ?: renderStrokes(strokes, size, ink)
                    if (bmp == null) {
                        vm.tell(if (pad.initials) "Draw your initials first" else "Draw your signature first")
                    } else {
                        vm.finishSignature(pad, bmp, save)
                    }
                },
            ) { Text("Done", color = c.accent, fontWeight = FontWeight.SemiBold) }
        }
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            TabChip("Draw", imported == null) { vm.importedSignature = null }
            TabChip("Image", imported != null) { onPickImage() }
        }
        Box(
            Modifier.fillMaxWidth().weight(1f).padding(16.dp).clip(PdfShapes.Small).background(Color.White).border(1.dp, c.divider, PdfShapes.Small)
                .onSizeChanged { size = it },
        ) {
            if (imported != null) {
                Image(imported.asImageBitmap(), null, modifier = Modifier.fillMaxSize().padding(16.dp))
            } else {
                Canvas(
                    Modifier.fillMaxSize().pointerInput(Unit) {
                        detectDragGestures(
                            onDragStart = { strokes.add(mutableStateListOf(it)) },
                        ) { change, _ ->
                            change.consume()
                            strokes.lastOrNull()?.add(change.position)
                        }
                    },
                ) {
                    // the line to sign on
                    drawLine(Color(0xFFBBBBBB), Offset(32f, size.height * 0.7f), Offset(this.size.width - 32f, this.size.height * 0.7f), 2f)
                    for (s in strokes) {
                        if (s.size < 2) continue
                        val p = Path()
                        s.forEachIndexed { i, o -> if (i == 0) p.moveTo(o.x, o.y) else p.lineTo(o.x, o.y) }
                        drawPath(p, ink, style = Stroke(width = 3.dp.toPx(), cap = StrokeCap.Round, join = StrokeJoin.Round))
                    }
                }
                if (strokes.isEmpty()) {
                    Text(if (pad.initials) "Draw your initials here" else "Draw your signature here", color = Color(0xFF8A8A8A), modifier = Modifier.align(Alignment.Center))
                }
            }
            Text("Clear", color = c.accent, modifier = Modifier.align(Alignment.TopEnd).clickable { strokes.clear(); vm.importedSignature = null }.padding(12.dp))
        }
        Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Text("Save to device", color = c.text, fontSize = 15.sp, modifier = Modifier.weight(1f))
            Switch(checked = save, onCheckedChange = { save = it }, colors = SwitchDefaults.colors(checkedTrackColor = c.accent))
        }
        Spacer(Modifier.height(8.dp))
    }
    SheetDivider(inset = 0.dp)
}

@Composable
private fun TabChip(label: String, selected: Boolean, onClick: () -> Unit) {
    val c = pdfColors
    Text(
        label,
        color = if (selected) c.accent else c.text,
        fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal,
        modifier = Modifier.clip(RoundedCornerShape(16.dp)).background(if (selected) c.pressed else Color.Transparent).clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 8.dp),
    )
}

/** The drawn strokes as a transparent PNG-ready bitmap, cropped to the ink; null when nothing was drawn. */
private fun renderStrokes(strokes: List<List<Offset>>, size: IntSize, color: Color): Bitmap? {
    val pts = strokes.flatten()
    if (pts.size < 2 || size.width == 0) return null
    val pad = 12f
    val minX = (pts.minOf { it.x } - pad).coerceAtLeast(0f)
    val minY = (pts.minOf { it.y } - pad).coerceAtLeast(0f)
    val maxX = (pts.maxOf { it.x } + pad).coerceAtMost(size.width.toFloat())
    val maxY = (pts.maxOf { it.y } + pad).coerceAtMost(size.height.toFloat())
    val w = (maxX - minX).toInt().coerceAtLeast(1)
    val h = (maxY - minY).toInt().coerceAtLeast(1)
    val bmp = Bitmap.createBitmap(w, h, Bitmap.Config.ARGB_8888)
    val canvas = android.graphics.Canvas(bmp)
    val paint = android.graphics.Paint().apply {
        isAntiAlias = true
        style = android.graphics.Paint.Style.STROKE
        strokeWidth = 6f
        strokeCap = android.graphics.Paint.Cap.ROUND
        strokeJoin = android.graphics.Paint.Join.ROUND
        this.color = android.graphics.Color.argb((color.alpha * 255).toInt(), (color.red * 255).toInt(), (color.green * 255).toInt(), (color.blue * 255).toInt())
    }
    for (s in strokes) {
        if (s.size < 2) continue
        val p = android.graphics.Path()
        s.forEachIndexed { i, o -> if (i == 0) p.moveTo(o.x - minX, o.y - minY) else p.lineTo(o.x - minX, o.y - minY) }
        canvas.drawPath(p, paint)
    }
    return bmp
}

/**
 * The menu over a selected Fill & Sign element: smaller, bigger, colour and delete (the
 * reference's element menu, without its convert-to-another-element step).
 */
@Composable
fun androidx.compose.foundation.layout.BoxScope.FillElementMenu(vm: AppViewModel, doc: OpenDocument, c: io.github.artcraftandroid.pdfcraft.core.CommentInfo) {
    val colors = pdfColors
    if (c.rect.size < 4) return
    var palette by remember { mutableStateOf(false) }
    val page = c.page.toInt()
    val a = doc.viewer.pageToScreen(page, c.rect[0], c.rect[1])
    val b = doc.viewer.pageToScreen(page, c.rect[2], c.rect[3])
    androidx.compose.foundation.Canvas(Modifier.fillMaxSize()) {
        drawRect(colors.accent, topLeft = Offset(a.x - 4f, a.y - 4f), size = androidx.compose.ui.geometry.Size(b.x - a.x + 8f, b.y - a.y + 8f), style = Stroke(2.dp.toPx()))
    }
    val density = androidx.compose.ui.platform.LocalDensity.current
    val menuY = with(density) { (a.y - 60.dp.toPx()).coerceAtLeast(doc.viewer.padTop) }
    Column(Modifier.offset { androidx.compose.ui.unit.IntOffset(a.x.toInt().coerceAtLeast(8), menuY.toInt()) }) {
        Row(
            Modifier.floatingSurface(PdfShapes.Medium, tint = colors.surface, alpha = GLASS_PANEL_ALPHA).padding(horizontal = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            BarIcon(LucideIcons.AArrowDown, "Smaller", { vm.scaleFill(doc, c, 1f / 1.2f) })
            BarIcon(LucideIcons.AArrowUp, "Bigger", { vm.scaleFill(doc, c, 1.2f) })
            BarIcon(LucideIcons.Palette, "Colour", { palette = !palette }, selected = palette)
            BarIcon(LucideIcons.Trash2, "Delete", { vm.selectedFill = null; vm.deleteComment(doc, c) }, tint = colors.red)
        }
        if (palette) {
            Row(Modifier.padding(top = 6.dp).floatingSurface(PdfShapes.Medium, tint = colors.surface, alpha = GLASS_PANEL_ALPHA).padding(8.dp)) {
                for (col in listOf(Color.Black, Color(0xFF0000FF), Color(0xFFE52237), Color(0xFF6AD928))) {
                    Box(Modifier.padding(4.dp).size(28.dp).clip(androidx.compose.foundation.shape.CircleShape).background(col).clickable {
                        palette = false
                        vm.toolPrefs.setFillColor(col)
                        vm.recolorComment(doc, c, col)
                    })
                }
            }
        }
    }
}
