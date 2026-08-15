package com.lanpulse.mobile.android

import android.Manifest
import android.app.AlertDialog
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.RectF
import android.graphics.drawable.GradientDrawable
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.TextView
import androidx.activity.ComponentActivity
import androidx.activity.OnBackPressedCallback
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import com.google.zxing.BinaryBitmap
import com.google.zxing.ChecksumException
import com.google.zxing.FormatException
import com.google.zxing.NotFoundException
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import com.lanpulse.mobile.PairingScanEvent
import com.lanpulse.mobile.MobileLanguage
import com.lanpulse.mobile.MobileStrings
import com.lanpulse.mobile.parsePairingCode
import java.util.concurrent.Executor
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.math.min
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.receiveAsFlow

internal object AndroidPairingScanner {
    private val channel = Channel<PairingScanEvent>(Channel.BUFFERED)

    val events: Flow<PairingScanEvent> = channel.receiveAsFlow()

    fun publish(event: PairingScanEvent) {
        channel.trySend(event)
    }
}

class LanPulseQrScannerActivity : ComponentActivity() {
    private val cameraExecutor = Executors.newSingleThreadExecutor()
    private val mainExecutor = Executor { command ->
        Handler(Looper.getMainLooper()).post(command)
    }
    private val completed = AtomicBoolean(false)
    private val qrReader = QRCodeReader()
    private var luminanceBuffer = ByteArray(0)

    private lateinit var previewView: PreviewView
    private lateinit var statusView: TextView
    private var cameraStarted = false
    private val strings: MobileStrings by lazy {
        MobileLanguage.fromCode(intent.getStringExtra(EXTRA_LANGUAGE)).strings()
    }

    private val requestCameraPermission = registerForActivityResult(
        ActivityResultContracts.RequestPermission(),
    ) { granted ->
        if (granted) startCamera() else showPermissionDialog()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        AndroidDiagnosticLog.initialize(applicationContext)
        AndroidDiagnosticLog.event("pairing_scanner_created")
        window.statusBarColor = Color.BLACK
        window.navigationBarColor = Color.BLACK
        setContentView(scannerView())

        onBackPressedDispatcher.addCallback(
            this,
            object : OnBackPressedCallback(true) {
                override fun handleOnBackPressed() = finish()
            },
        )

        if (checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
            startCamera()
        } else {
            requestCameraPermission.launch(Manifest.permission.CAMERA)
        }
    }

    override fun onResume() {
        super.onResume()
        if (
            !cameraStarted &&
            checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED
        ) {
            startCamera()
        }
    }

    override fun onDestroy() {
        AndroidDiagnosticLog.event("pairing_scanner_destroyed", "completed=${completed.get()}")
        cameraExecutor.shutdown()
        super.onDestroy()
    }

    private fun startCamera() {
        if (cameraStarted || isFinishing || completed.get()) return
        AndroidDiagnosticLog.event("pairing_camera_starting")
        cameraStarted = true
        statusView.text = strings.lookingForPairingCode

        val providerFuture = ProcessCameraProvider.getInstance(this)
        providerFuture.addListener(
            {
                runCatching {
                    val provider = providerFuture.get()
                    val preview = Preview.Builder().build().also {
                        it.surfaceProvider = previewView.surfaceProvider
                    }
                    val analysis = ImageAnalysis.Builder()
                        .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                        .build()
                        .also { useCase ->
                            useCase.setAnalyzer(cameraExecutor, ::analyzeImage)
                        }
                    provider.unbindAll()
                    provider.bindToLifecycle(
                        this,
                        CameraSelector.DEFAULT_BACK_CAMERA,
                        preview,
                        analysis,
                    )
                }.onFailure { error ->
                    AndroidDiagnosticLog.error("pairing_camera_failed", error)
                    failAndFinish(error.message ?: strings.cameraUnavailable)
                }
            },
            mainExecutor,
        )
    }

    private fun analyzeImage(imageProxy: ImageProxy) {
        try {
            if (completed.get()) return
            val rawValue = decodeQrCode(imageProxy) ?: return
            val pairing = parsePairingCode(rawValue)
            if (pairing != null && completed.compareAndSet(false, true)) {
                AndroidDiagnosticLog.event(
                    "pairing_code_found",
                    "control_url=${pairing.controlUrl}",
                )
                AndroidPairingScanner.publish(PairingScanEvent.Found(pairing))
                runOnUiThread { finish() }
            } else if (pairing == null) {
                runOnUiThread { statusView.text = strings.notPairingCode }
            }
        } catch (_: Exception) {
            runOnUiThread { statusView.text = strings.unableToReadCode }
        } finally {
            qrReader.reset()
            imageProxy.close()
        }
    }

    private fun decodeQrCode(imageProxy: ImageProxy): String? {
        val plane = imageProxy.planes.firstOrNull() ?: return null
        val width = imageProxy.width
        val height = imageProxy.height
        val requiredBytes = width * height
        if (luminanceBuffer.size != requiredBytes) luminanceBuffer = ByteArray(requiredBytes)

        val sourceBuffer = plane.buffer
        val base = sourceBuffer.position()
        val rowStride = plane.rowStride
        val pixelStride = plane.pixelStride
        var output = 0
        for (row in 0 until height) {
            var input = base + row * rowStride
            repeat(width) {
                luminanceBuffer[output++] = sourceBuffer.get(input)
                input += pixelStride
            }
        }

        val source = PlanarYUVLuminanceSource(
            luminanceBuffer,
            width,
            height,
            0,
            0,
            width,
            height,
            false,
        )
        return try {
            qrReader.decode(BinaryBitmap(HybridBinarizer(source))).text
        } catch (_: NotFoundException) {
            null
        } catch (_: ChecksumException) {
            null
        } catch (_: FormatException) {
            null
        }
    }

    private fun showPermissionDialog() {
        AndroidDiagnosticLog.event("pairing_camera_permission_denied")
        statusView.text = strings.cameraPermissionRequired
        AlertDialog.Builder(this)
            .setTitle(strings.cameraPermission)
            .setMessage(strings.cameraPermissionMessage)
            .setPositiveButton(strings.openSettings) { _, _ ->
                startActivity(
                    Intent(
                        Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                        Uri.parse("package:$packageName"),
                    ),
                )
            }
            .setNegativeButton(strings.cancel) { _, _ -> finish() }
            .show()
    }

    private fun failAndFinish(message: String) {
        if (!completed.compareAndSet(false, true)) return
        AndroidDiagnosticLog.event("pairing_scanner_failed", "message=$message")
        AndroidPairingScanner.publish(PairingScanEvent.Failed(message))
        runOnUiThread { finish() }
    }

    private fun scannerView(): View {
        val root = FrameLayout(this).apply {
            setBackgroundColor(Color.BLACK)
        }
        previewView = PreviewView(this).apply {
            implementationMode = PreviewView.ImplementationMode.PERFORMANCE
            scaleType = PreviewView.ScaleType.FILL_CENTER
        }
        root.addView(
            previewView,
            FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT),
        )
        root.addView(
            ScannerOverlayView(this),
            FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT),
        )

        val title = TextView(this).apply {
            text = strings.scanPairingCode
            setTextColor(Color.WHITE)
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 19f)
            setTypeface(typeface, android.graphics.Typeface.BOLD)
            gravity = Gravity.CENTER_VERTICAL
        }
        root.addView(
            title,
            FrameLayout.LayoutParams(MATCH_PARENT, dp(56)).apply {
                gravity = Gravity.TOP
                marginStart = dp(20)
                marginEnd = dp(76)
                topMargin = dp(8)
            },
        )

        val closeButton = ImageButton(this).apply {
            setImageResource(android.R.drawable.ic_menu_close_clear_cancel)
            setColorFilter(Color.WHITE)
            background = roundedBackground(Color.argb(190, 18, 24, 21), 6f)
            contentDescription = strings.closeScanner
            setPadding(dp(12), dp(12), dp(12), dp(12))
            setOnClickListener { finish() }
        }
        root.addView(
            closeButton,
            FrameLayout.LayoutParams(dp(46), dp(46)).apply {
                gravity = Gravity.TOP or Gravity.END
                topMargin = dp(13)
                marginEnd = dp(18)
            },
        )

        statusView = TextView(this).apply {
            text = strings.lookingForPairingCode
            setTextColor(Color.WHITE)
            setTextSize(TypedValue.COMPLEX_UNIT_SP, 14f)
            gravity = Gravity.CENTER
            setPadding(dp(16), 0, dp(16), 0)
            background = roundedBackground(Color.argb(210, 18, 24, 21), 6f)
        }
        root.addView(
            statusView,
            FrameLayout.LayoutParams(MATCH_PARENT, dp(48)).apply {
                gravity = Gravity.BOTTOM
                marginStart = dp(20)
                marginEnd = dp(20)
                bottomMargin = dp(28)
            },
        )
        return root
    }

    private fun roundedBackground(color: Int, radiusDp: Float) = GradientDrawable().apply {
        setColor(color)
        cornerRadius = dp(radiusDp).toFloat()
    }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()
    private fun dp(value: Float): Int = (value * resources.displayMetrics.density).toInt()

    companion object {
        const val EXTRA_LANGUAGE = "language"

        private const val MATCH_PARENT = FrameLayout.LayoutParams.MATCH_PARENT
    }
}

private class ScannerOverlayView(context: android.content.Context) : View(context) {
    private val shade = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.argb(145, 0, 0, 0)
        style = Paint.Style.FILL
    }
    private val corners = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        color = Color.rgb(84, 213, 155)
        strokeWidth = 4f * resources.displayMetrics.density
        strokeCap = Paint.Cap.SQUARE
        style = Paint.Style.STROKE
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        val frameSize = min(width * 0.74f, height * 0.48f)
        val left = (width - frameSize) / 2f
        val top = (height - frameSize) / 2f
        val frame = RectF(left, top, left + frameSize, top + frameSize)

        canvas.drawRect(0f, 0f, width.toFloat(), frame.top, shade)
        canvas.drawRect(0f, frame.bottom, width.toFloat(), height.toFloat(), shade)
        canvas.drawRect(0f, frame.top, frame.left, frame.bottom, shade)
        canvas.drawRect(frame.right, frame.top, width.toFloat(), frame.bottom, shade)

        val corner = 28f * resources.displayMetrics.density
        drawCorner(canvas, frame.left, frame.top, corner, 1f, 1f)
        drawCorner(canvas, frame.right, frame.top, corner, -1f, 1f)
        drawCorner(canvas, frame.left, frame.bottom, corner, 1f, -1f)
        drawCorner(canvas, frame.right, frame.bottom, corner, -1f, -1f)
    }

    private fun drawCorner(
        canvas: Canvas,
        x: Float,
        y: Float,
        length: Float,
        horizontalDirection: Float,
        verticalDirection: Float,
    ) {
        canvas.drawLine(x, y, x + length * horizontalDirection, y, corners)
        canvas.drawLine(x, y, x, y + length * verticalDirection, corners)
    }
}
