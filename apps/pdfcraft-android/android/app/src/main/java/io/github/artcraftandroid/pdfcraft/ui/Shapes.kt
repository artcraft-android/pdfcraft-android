package io.github.artcraftandroid.pdfcraft.ui

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.unit.dp

/**
 * The corner radii the app's surfaces share, so a button, a card, a menu and a dialog read as one family
 * with the floating bubbles ([Bubble] is the largest).
 */
object PdfShapes {
    /** Check boxes and other small marks. */
    val Mark = RoundedCornerShape(4.dp)

    /** Page and file thumbnails. */
    val Thumb = RoundedCornerShape(8.dp)

    /** Icon buttons, fields, tool tiles and the rows' pressed backgrounds. */
    val Small = RoundedCornerShape(12.dp)

    /** Cards, menus and pop-ups. */
    val Medium = RoundedCornerShape(16.dp)

    /** Dialogs. */
    val Large = RoundedCornerShape(28.dp)

    /** The floating bars (a capsule for a one-row bar). */
    val Bubble = RoundedCornerShape(30.dp)

    /** Buttons and chips: a capsule. */
    val Pill = RoundedCornerShape(50)
}
