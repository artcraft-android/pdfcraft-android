package io.github.artcraftandroid.pdfcraft.ui

import android.content.Context
import android.provider.Settings
import androidx.compose.ui.MotionDurationScale

/**
 * How fast the app's animations run. Every Compose animation (sheets, bars, fades, the page
 * settling, zoom) reads this from its coroutine, because [MainActivity] puts it in the window's
 * recomposer: Android's animator duration scale as usual ("Remove animations" makes it 0), and 0
 * in E-ink mode (#13), where every animation snaps to its end.
 */
object AppMotion : MotionDurationScale {
    /** E-ink mode is on. */
    @Volatile
    var eInk = false

    /** Android's animator duration scale, read when the app comes to the front. */
    @Volatile
    var system = 1f
        private set

    override val scaleFactor: Float get() = if (eInk) 0f else system

    fun readSystem(context: Context) {
        system = runCatching { Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) }.getOrDefault(1f)
    }
}
