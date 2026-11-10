package io.github.artcraftandroid.pdfcraft.ui.viewer

import kotlin.math.max
import kotlin.math.min

/** A page's displayed size in PDF points. */
data class PageSize(val width: Float, val height: Float)

/** A rectangle in layout pixels (the document at zoom 1). */
data class LRect(val left: Float, val top: Float, val right: Float, val bottom: Float) {
    val width get() = right - left
    val height get() = bottom - top
    fun contains(x: Float, y: Float) = x in left..right && y in top..bottom
}

/** How pages follow each other. */
enum class PageMode {
    /** One column, scrolled vertically; every page as wide as the widest fits the screen. */
    Continuous,

    /** One page at a time, side by side, each fitted to the screen; swipes go page to page. */
    SinglePage,
}

/**
 * Continuous pages in pairs, side by side (View settings, Two pages: #59). The reference offers
 * both on tablets.
 */
enum class Spread {
    /** One page to a row. */
    None,

    /** Two pages to a row: 1 and 2, 3 and 4, and so on. */
    TwoPages,

    /** The first page alone on the right, as a book's cover, then 2 and 3, 4 and 5, and so on. */
    TwoPagesCover,
}

/**
 * Where each page is at zoom 1, in layout pixels, for a viewport of [viewW] × [viewH] pixels.
 * Pure geometry, so it can be tested off the device.
 *
 * Continuous: the pages stack in one column with [gap] between them, scaled together so the
 * widest fills the width less [margin] on each side; [padTop] and [padBottom] keep the first and
 * last pages clear of the bars. With a [spread], they go two to a row instead, on either side of
 * a [gap] down the middle, scaled so two of the widest fill the width; a row's pages share its
 * top. SinglePage: each page is fitted into the viewport (less the margins and paddings) and
 * centred in a slot one viewport wide.
 */
class DocLayout(
    val pages: List<PageSize>,
    val mode: PageMode,
    val viewW: Float,
    val viewH: Float,
    val gap: Float,
    val margin: Float,
    val padTop: Float,
    val padBottom: Float,
    val spread: Spread = Spread.None,
) {
    val rects: List<LRect>
    val width: Float
    val height: Float

    /** For each page, the first page of its row (itself, unless it's a spread's right page). */
    private val rowStart = IntArray(pages.size) { it }

    /** For each page, the bottom of its row (the taller page's). */
    private val rowBottom = FloatArray(pages.size)

    init {
        val out = ArrayList<LRect>(pages.size)
        when (mode) {
            PageMode.Continuous -> {
                val widest = pages.maxOfOrNull { it.width }?.coerceAtLeast(1f) ?: 1f
                val two = spread != Spread.None
                val scale = if (two) max(1f, viewW - 2 * margin - gap) / 2f / widest else max(1f, viewW - 2 * margin) / widest
                val middle = viewW / 2f
                var y = padTop
                var i = 0
                while (i < pages.size) {
                    // the cover sits alone in the right column; otherwise pages pair up
                    val cover = spread == Spread.TwoPagesCover && i == 0
                    val n = if (!two || cover) 1 else min(2, pages.size - i)
                    var rowH = 0f
                    for (k in 0 until n) {
                        val p = pages[i + k]
                        val w = p.width.coerceAtLeast(1f) * scale
                        val h = p.height.coerceAtLeast(1f) * scale
                        val x = when {
                            !two -> (viewW - w) / 2f
                            cover || k == 1 -> middle + gap / 2f
                            else -> middle - gap / 2f - w
                        }
                        out.add(LRect(x, y, x + w, y + h))
                        rowH = max(rowH, h)
                    }
                    for (k in 0 until n) {
                        rowStart[i + k] = i
                        rowBottom[i + k] = y + rowH
                    }
                    y += rowH + gap
                    i += n
                }
                width = viewW
                height = if (pages.isEmpty()) viewH else y - gap + padBottom
            }
            PageMode.SinglePage -> {
                val availW = max(1f, viewW - 2 * margin)
                val availH = max(1f, viewH - padTop - padBottom - 2 * margin)
                pages.forEachIndexed { i, p ->
                    val s = min(availW / p.width.coerceAtLeast(1f), availH / p.height.coerceAtLeast(1f))
                    val w = p.width.coerceAtLeast(1f) * s
                    val h = p.height.coerceAtLeast(1f) * s
                    val x = i * viewW + (viewW - w) / 2f
                    val y = padTop + margin + (availH - h) / 2f
                    out.add(LRect(x, y, x + w, y + h))
                    rowBottom[i] = y + h
                }
                width = max(viewW, pages.size * viewW)
                height = viewH
            }
        }
        rects = out
    }

    /** Pixels per PDF point of page [i] at zoom 1. */
    fun scaleOf(i: Int): Float = rects[i].width / pages[i].width.coerceAtLeast(1f)

    /** The pages that overlap layout rows [top]..[bottom] (and columns [left]..[right]). */
    fun visible(left: Float, top: Float, right: Float, bottom: Float): IntRange {
        if (rects.isEmpty()) return IntRange.EMPTY
        return when (mode) {
            PageMode.Continuous -> {
                val first = firstAtOrBelow(top)
                var last = first
                while (last + 1 < rects.size && rects[last + 1].top <= bottom) last++
                first..last
            }
            PageMode.SinglePage -> {
                val first = (left / viewW).toInt().coerceIn(0, rects.lastIndex)
                val last = (right / viewW).toInt().coerceIn(0, rects.lastIndex)
                first..last
            }
        }
    }

    /** The first page of the first row whose bottom is at or below layout row [y] (binary search). */
    private fun firstAtOrBelow(y: Float): Int {
        var lo = 0
        var hi = rects.lastIndex
        while (lo < hi) {
            val mid = (lo + hi) / 2
            if (rowBottom[mid] + gap < y) lo = mid + 1 else hi = mid
        }
        return rowStart[lo]
    }

    /** The last page of the row that starts at page [first]. */
    private fun rowEnd(first: Int): Int {
        var e = first
        while (e + 1 < rects.size && rowStart[e + 1] == first) e++
        return e
    }

    /** In the row that starts at page [first], the page nearest layout column [x]. */
    private fun nearestInRow(first: Int, x: Float): Int {
        var best = first
        var bestD = Float.MAX_VALUE
        for (k in first..rowEnd(first)) {
            val r = rects[k]
            val d = if (x < r.left) r.left - x else if (x > r.right) x - r.right else 0f
            if (d < bestD) {
                best = k
                bestD = d
            }
        }
        return best
    }

    /**
     * The page to go to for the next ([dir] > 0) or previous page (Page Down, Page Up): the first
     * page of the row after or before page [i]'s, so a spread moves a whole row.
     */
    fun stepRow(i: Int, dir: Int): Int {
        if (rects.isEmpty()) return 0
        val first = rowStart[i.coerceIn(0, rects.lastIndex)]
        return when {
            dir > 0 -> (rowEnd(first) + 1).coerceAtMost(rects.lastIndex)
            dir < 0 && first > 0 -> rowStart[first - 1]
            else -> first
        }
    }

    /** The page under layout point ([x], [y]), if any. */
    fun pageAt(x: Float, y: Float): Int? {
        val range = visible(x, y, x, y)
        return range.firstOrNull { rects[it].contains(x, y) }
    }

    /** The page nearest layout point ([x], [y]): the one under it, else the closest. */
    fun nearestPage(x: Float, y: Float): Int {
        if (rects.isEmpty()) return 0
        return when (mode) {
            PageMode.Continuous -> {
                val i = firstAtOrBelow(y)
                val top = rects[i].top
                val bottom = rowBottom[i]
                val next = rowEnd(i) + 1
                val row = when {
                    // in the gap below row i: whichever edge is closer
                    y > bottom && next < rects.size -> if (rects[next].top - y < y - bottom) next else i
                    y < top && i > 0 -> if (top - y > y - rowBottom[i - 1]) rowStart[i - 1] else i
                    else -> i
                }
                nearestInRow(row, x)
            }
            PageMode.SinglePage -> (x / viewW).toInt().coerceIn(0, rects.lastIndex)
        }
    }
}
