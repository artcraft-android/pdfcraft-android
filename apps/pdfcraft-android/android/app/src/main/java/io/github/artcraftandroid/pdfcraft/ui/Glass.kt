package io.github.artcraftandroid.pdfcraft.ui

import android.os.Build
import androidx.compose.foundation.border
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.BlurEffect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.TileMode
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.layer.GraphicsLayer
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.compose.ui.graphics.rememberGraphicsLayer
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInWindow
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import kotlin.math.roundToInt

/**
 * What the floating surfaces blur: the screen's content (the document, or Home), drawn once into a
 * layer. A surface draws a blurred copy of the part of it behind it (frosted glass). [origin] is where
 * the content sits in the window.
 */
@Stable
class GlassSource(val layer: GraphicsLayer) {
    var origin by mutableStateOf(Offset.Zero)
}

val LocalGlassSource = staticCompositionLocalOf<GlassSource?> { null }

@Composable
fun rememberGlassSource(): GlassSource {
    val layer = rememberGraphicsLayer()
    return remember(layer) { GlassSource(layer) }
}

/** Mark content as what the floating surfaces frost: it is recorded into [source] as it draws. */
fun Modifier.glassSource(source: GlassSource): Modifier = this
    .onGloballyPositioned { source.origin = it.positionInWindow() }
    .drawWithContent {
        source.layer.record { this@drawWithContent.drawContent() }
        drawLayer(source.layer)
    }

/** Floating surfaces sit this far in from the screen's sides... */
val FloatSide = 12.dp

/** ...and this far from the status band above and the navigation area below. */
val FloatEdge = 8.dp

/** How much of the surface tint covers the blurred content behind a bar (the content still reads through). */
const val GLASS_BAR_ALPHA = 0.6f

/** The same for a sheet or a menu, whose text must stay readable over a busy page. */
const val GLASS_PANEL_ALPHA = 0.84f

/**
 * A surface floating above the content: rounded, lifted off it by a soft shadow and a hairline edge, and
 * frosted: the content behind it shows blurred through a translucent [tint] (Android 12 and up; older
 * versions get a nearly opaque tint, as the platform cannot blur what is behind).
 */
@Composable
fun Modifier.floatingSurface(
    shape: Shape = PdfShapes.Bubble,
    tint: Color = pdfColors.surface,
    alpha: Float = GLASS_BAR_ALPHA,
): Modifier {
    val c = pdfColors
    val glass = LocalGlassSource.current
    val blurs = glass != null && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
    val frost = rememberGraphicsLayer()
    var at by remember { mutableStateOf(Offset.Zero) }
    val sigma = with(LocalDensity.current) { 20.dp.toPx() }
    return this
        .shadow(14.dp, shape, ambientColor = Color.Black.copy(alpha = 0.25f), spotColor = Color.Black.copy(alpha = 0.35f))
        .clip(shape)
        .onGloballyPositioned { at = it.positionInWindow() }
        .drawBehind {
            if (blurs && glass != null) {
                // The copy is recorded wider than the surface by the blur's reach, so the blur at its
                // edges sees the content there; blurred on its own the edge pixels are smeared inward,
                // and every line of text passing an edge flashes a streak.
                val reach = (sigma * 3f).roundToInt()
                frost.renderEffect = BlurEffect(sigma, sigma, TileMode.Clamp)
                frost.record(size = IntSize(size.width.roundToInt() + 2 * reach, size.height.roundToInt() + 2 * reach)) {
                    translate(glass.origin.x - at.x + reach, glass.origin.y - at.y + reach) { drawLayer(glass.layer) }
                }
                translate(-reach.toFloat(), -reach.toFloat()) { drawLayer(frost) }
                drawRect(tint.copy(alpha = alpha))
            } else {
                drawRect(tint.copy(alpha = 0.97f))
            }
        }
        .border(0.5.dp, c.divider.copy(alpha = 0.7f), shape)
}
