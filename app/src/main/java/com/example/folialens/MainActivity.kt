package com.example.folialens

import android.Manifest
import android.content.pm.PackageManager
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview as CameraPreview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.core.content.ContextCompat
import com.example.folialens.core.CameraZeroCopyAnalyzer
import com.example.folialens.core.LiteRtClassifier
import com.example.folialens.core.LiveAudioAnalysisResult
import com.example.folialens.core.LiveAudioDSPController
import com.example.folialens.core.LiveVisionAnalysisResult
import com.example.folialens.core.NativeBridge
import com.example.folialens.core.uniffi.AtlasEngine
import com.example.folialens.core.uniffi.Kingdom
import com.example.folialens.core.uniffi.SafetyStatus
import com.example.folialens.core.uniffi.TaxonDetail
import com.example.folialens.core.uniffi.TaxonSummary
import com.example.folialens.core.uniffi.rustHealthCheck
import com.example.folialens.ui.theme.FoliaLensTheme
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.Executors

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            FoliaLensTheme {
                MainAppScaffold()
            }
        }
    }
}

enum class NavigationTab(val title: String, val icon: String) {
    ATLAS("Atlas", "📖"),
    VISION("Live Vision", "📷"),
    ACOUSTIC("Acoustic Radar", "🎙️"),
}

data class BridgeStatus(
    val jniResult: String,
    val jniSuccess: Boolean,
    val uniffiResult: String,
    val uniffiSuccess: Boolean,
)

@Composable
fun MainAppScaffold() {
    val context = LocalContext.current
    var selectedTab by remember { mutableStateOf(NavigationTab.ATLAS) }

    // Core Atlas Engine and LiteRT Classifier
    val atlasEngine = remember {
        try {
            AtlasEngine.newInMemoryDemo()
        } catch (_: Throwable) {
            null
        }
    }
    val classifier = remember { LiteRtClassifier(context) }

    DisposableEffect(Unit) {
        onDispose {
            classifier.close()
        }
    }

    var selectedTaxonDetail by remember { mutableStateOf<TaxonDetail?>(null) }

    Scaffold(
        modifier = Modifier.fillMaxSize(),
        bottomBar = {
            NavigationBar {
                NavigationTab.entries.forEach { tab ->
                    NavigationBarItem(
                        selected = selectedTab == tab,
                        onClick = { selectedTab = tab },
                        icon = {
                            Text(text = tab.icon, style = MaterialTheme.typography.titleMedium)
                        },
                        label = { Text(tab.title) }
                    )
                }
            }
        }
    ) { innerPadding ->
        Box(modifier = Modifier.padding(innerPadding)) {
            when (selectedTab) {
                NavigationTab.ATLAS -> AtlasTabContent(
                    atlasEngine = atlasEngine,
                    onTaxonSelected = { taxon ->
                        atlasEngine?.let { selectedTaxonDetail = it.getTaxonDetail(taxon.taxonId) }
                    }
                )
                NavigationTab.VISION -> LiveVisionTabContent(
                    atlasEngine = atlasEngine,
                    classifier = classifier,
                    onTaxonSelected = { taxon ->
                        atlasEngine?.let { selectedTaxonDetail = it.getTaxonDetail(taxon.taxonId) }
                    }
                )
                NavigationTab.ACOUSTIC -> AcousticRadarTabContent(
                    atlasEngine = atlasEngine,
                    classifier = classifier,
                    onTaxonSelected = { taxon ->
                        atlasEngine?.let { selectedTaxonDetail = it.getTaxonDetail(taxon.taxonId) }
                    }
                )
            }

            // Modal dialog for taxon details (uncompressed Zstd Markdown)
            selectedTaxonDetail?.let { detail ->
                TaxonDetailDialog(
                    detail = detail,
                    onDismiss = { selectedTaxonDetail = null }
                )
            }
        }
    }
}

// ============================================================================
// TAB 1: ATLAS & FTS5 SEARCH
// ============================================================================

@Composable
fun AtlasTabContent(
    atlasEngine: AtlasEngine?,
    onTaxonSelected: (TaxonSummary) -> Unit,
) {
    var searchQuery by remember { mutableStateOf("") }
    var selectedKingdomFilter by remember { mutableStateOf<Kingdom?>(null) }
    var searchResults by remember { mutableStateOf<List<TaxonSummary>>(emptyList()) }

    // Health checks
    val bridgeStatus = remember {
        val jniOk = try { NativeBridge.jniHealthCheck(); true } catch (_: Throwable) { false }
        val uniffiOk = try { rustHealthCheck(); true } catch (_: Throwable) { false }
        val jRes = if (jniOk) "Direct JNI: OK" else "Direct JNI: Error"
        val uRes = if (uniffiOk) "UniFFI: OK" else "UniFFI: Error"
        BridgeStatus(jRes, jniOk, uRes, uniffiOk)
    }

    LaunchedEffect(atlasEngine, searchQuery) {
        if (atlasEngine != null) {
            try {
                searchResults = atlasEngine.searchTaxa(searchQuery, 20u)
            } catch (_: Throwable) {}
        }
    }

    val filteredResults = searchResults.filter { taxon ->
        selectedKingdomFilter == null || taxon.kingdom == selectedKingdomFilter
    }

    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .padding(horizontal = 16.dp),
        contentPadding = PaddingValues(vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp)
    ) {
        item {
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
                elevation = CardDefaults.cardElevation(defaultElevation = 2.dp)
            ) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(14.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween
                ) {
                    Column {
                        Text(
                            text = "🌿 FoliaLens Offline Atlas",
                            style = MaterialTheme.typography.titleMedium,
                            fontWeight = FontWeight.Bold
                        )
                        Text(
                            text = "${bridgeStatus.jniResult} | ${bridgeStatus.uniffiResult}",
                            style = MaterialTheme.typography.bodySmall,
                            fontFamily = FontFamily.Monospace
                        )
                    }
                    Box(
                        modifier = Modifier
                            .size(12.dp)
                            .background(
                                color = if (bridgeStatus.jniSuccess && bridgeStatus.uniffiSuccess && atlasEngine != null) Color(0xFF2E7D32) else Color(0xFFC62828),
                                shape = CircleShape
                            )
                    )
                }
            }
        }

        item {
            OutlinedTextField(
                value = searchQuery,
                onValueChange = { searchQuery = it },
                modifier = Modifier.fillMaxWidth(),
                placeholder = { Text("Поиск: Мухомор, Берёза, Лисичка...") },
                singleLine = true,
                shape = RoundedCornerShape(12.dp)
            )
        }

        item {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp)
            ) {
                FilterChip(
                    selected = selectedKingdomFilter == null,
                    onClick = { selectedKingdomFilter = null },
                    label = { Text("Все") }
                )
                FilterChip(
                    selected = selectedKingdomFilter == Kingdom.PLANTAE,
                    onClick = { selectedKingdomFilter = if (selectedKingdomFilter == Kingdom.PLANTAE) null else Kingdom.PLANTAE },
                    label = { Text("Флора") }
                )
                FilterChip(
                    selected = selectedKingdomFilter == Kingdom.FUNGI,
                    onClick = { selectedKingdomFilter = if (selectedKingdomFilter == Kingdom.FUNGI) null else Kingdom.FUNGI },
                    label = { Text("Грибы") }
                )
                FilterChip(
                    selected = selectedKingdomFilter == Kingdom.ANIMALIA,
                    onClick = { selectedKingdomFilter = if (selectedKingdomFilter == Kingdom.ANIMALIA) null else Kingdom.ANIMALIA },
                    label = { Text("Фауна") }
                )
            }
        }

        items(filteredResults) { taxon ->
            TaxonItemCard(taxon = taxon, onClick = { onTaxonSelected(taxon) })
        }
    }
}

// ============================================================================
// TAB 2: LIVE VISION AI (CameraX + Zero-Copy Rust + LiteRT)
// ============================================================================

@Composable
fun LiveVisionTabContent(
    atlasEngine: AtlasEngine?,
    classifier: LiteRtClassifier,
    onTaxonSelected: (TaxonSummary) -> Unit,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current

    var hasCameraPermission by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED
        )
    }

    val permissionLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.RequestPermission()
    ) { granted ->
        hasCameraPermission = granted
    }

    var analysisResult by remember { mutableStateOf<LiveVisionAnalysisResult?>(null) }

    val cameraExecutor = remember { Executors.newSingleThreadExecutor() }
    DisposableEffect(Unit) {
        onDispose {
            cameraExecutor.shutdown()
        }
    }

    Column(modifier = Modifier.fillMaxSize()) {
        if (!hasCameraPermission) {
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(24.dp),
                contentAlignment = Alignment.Center
            ) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(
                        text = "📷 Требуется разрешение на камеру",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold
                    )
                    Spacer(modifier = Modifier.height(8.dp))
                    Text(
                        text = "Камера используется для прямого захвата кадров в Zero-Copy Rust буферы без GC-пауз.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                    Spacer(modifier = Modifier.height(16.dp))
                    Button(onClick = { permissionLauncher.launch(Manifest.permission.CAMERA) }) {
                        Text("Предоставить доступ к Камере")
                    }
                }
            }
        } else if (atlasEngine != null) {
            // Live Viewfinder + HUD
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(1.2f)
            ) {
                AndroidView(
                    modifier = Modifier.fillMaxSize(),
                    factory = { ctx ->
                        val previewView = PreviewView(ctx)
                        val cameraProviderFuture = ProcessCameraProvider.getInstance(ctx)
                        cameraProviderFuture.addListener({
                            val cameraProvider = cameraProviderFuture.get()
                            val preview = CameraPreview.Builder().build().also {
                                it.surfaceProvider = previewView.surfaceProvider
                            }

                            val analyzer = CameraZeroCopyAnalyzer(
                                atlasEngine = atlasEngine,
                                classifier = classifier,
                                onResult = { result -> analysisResult = result }
                            )

                            val imageAnalysis = ImageAnalysis.Builder()
                                .setOutputImageFormat(ImageAnalysis.OUTPUT_IMAGE_FORMAT_YUV_420_888)
                                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                                .build()
                                .also {
                                    it.setAnalyzer(cameraExecutor, analyzer)
                                }

                            try {
                                cameraProvider.unbindAll()
                                cameraProvider.bindToLifecycle(
                                    lifecycleOwner,
                                    CameraSelector.DEFAULT_BACK_CAMERA,
                                    preview,
                                    imageAnalysis
                                )
                            } catch (_: Throwable) {}
                        }, ContextCompat.getMainExecutor(ctx))
                        previewView
                    }
                )

                // Center-Crop Aim Target Box
                Box(
                    modifier = Modifier
                        .size(240.dp)
                        .align(Alignment.Center)
                        .border(2.dp, Color.White.copy(alpha = 0.8f), RoundedCornerShape(16.dp))
                )

                // Top HUD Overlay
                Surface(
                    color = Color.Black.copy(alpha = 0.65f),
                    shape = RoundedCornerShape(8.dp),
                    modifier = Modifier
                        .align(Alignment.TopCenter)
                        .padding(12.dp)
                ) {
                    analysisResult?.let { res ->
                        Column(modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp)) {
                            Text(
                                text = "Rust YUV→RGB: ${"%.1f".format(res.rustPreprocessMs)} ms | LiteRT: ${"%.1f".format(res.inferenceMs)} ms",
                                style = MaterialTheme.typography.labelSmall,
                                fontFamily = FontFamily.Monospace,
                                color = Color(0xFF69F0AE),
                                fontWeight = FontWeight.Bold
                            )
                            Text(
                                text = "Mode: ${res.engineMode} | GPS: СПб H3 (Res 4)",
                                style = MaterialTheme.typography.labelSmall,
                                fontFamily = FontFamily.Monospace,
                                color = Color.White.copy(alpha = 0.85f)
                            )
                        }
                    } ?: Text(
                        text = "Zero-Copy Pipeline: Инициализация потока...",
                        style = MaterialTheme.typography.labelSmall,
                        fontFamily = FontFamily.Monospace,
                        color = Color.White,
                        modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp)
                    )
                }
            }

            // Bottom Results Section
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .weight(0.9f)
                    .background(MaterialTheme.colorScheme.surface)
                    .padding(14.dp)
            ) {
                Text(
                    text = "Распознано в видоискателе (Top-3):",
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.Bold
                )
                Spacer(modifier = Modifier.height(8.dp))

                val taxa = analysisResult?.topTaxa.orEmpty()
                if (taxa.isEmpty()) {
                    Text(
                        text = "Наведите камеру на объект флоры или грибов...",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant
                    )
                } else {
                    LazyColumn(
                        modifier = Modifier.fillMaxSize(),
                        verticalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        items(taxa) { taxon ->
                            TaxonItemCard(taxon = taxon, onClick = { onTaxonSelected(taxon) })
                        }
                    }
                }
            }
        }
    }
}

// ============================================================================
// TAB 3: LIVE ACOUSTIC RADAR (AudioRecord + rustfft + LiteRT)
// ============================================================================

@Composable
fun AcousticRadarTabContent(
    atlasEngine: AtlasEngine?,
    classifier: LiteRtClassifier,
    onTaxonSelected: (TaxonSummary) -> Unit,
) {
    val context = LocalContext.current
    val coroutineScope = rememberCoroutineScope()

    var hasAudioPermission by remember {
        mutableStateOf(
            ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED
        )
    }

    val permissionLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.RequestPermission()
    ) { granted ->
        hasAudioPermission = granted
    }

    var audioResult by remember { mutableStateOf<LiveAudioAnalysisResult?>(null) }

    val audioController = remember(atlasEngine) {
        if (atlasEngine != null) {
            LiveAudioDSPController(
                context = context,
                atlasEngine = atlasEngine,
                classifier = classifier,
                onResult = { res -> audioResult = res }
            )
        } else null
    }

    DisposableEffect(audioController) {
        onDispose {
            audioController?.stopListening()
        }
    }

    var isListening by remember { mutableStateOf(false) }

    LazyColumn(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(14.dp)
    ) {
        item {
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.tertiaryContainer),
                elevation = CardDefaults.cardElevation(defaultElevation = 2.dp)
            ) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Text(
                        text = "🎙️ Bioacoustic Radar (48 kHz PCM)",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                        color = MaterialTheme.colorScheme.onTertiaryContainer
                    )
                    Spacer(modifier = Modifier.height(4.dp))
                    Text(
                        text = "Скользящее окно 3с (144 000 сэмплов) → STFT (rustfft) → 96 Mel-фильтров → Magma RGBA.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onTertiaryContainer.copy(alpha = 0.85f)
                    )
                    Spacer(modifier = Modifier.height(14.dp))

                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp)
                    ) {
                        Button(
                            onClick = {
                                if (!hasAudioPermission) {
                                    permissionLauncher.launch(Manifest.permission.RECORD_AUDIO)
                                } else {
                                    if (isListening) {
                                        audioController?.stopListening()
                                        isListening = false
                                    } else {
                                        val started = audioController?.startListening(coroutineScope) ?: false
                                        isListening = started
                                    }
                                }
                            },
                            colors = ButtonDefaults.buttonColors(
                                containerColor = if (isListening) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary
                            ),
                            modifier = Modifier.weight(1f)
                        ) {
                            Text(if (isListening) "Остановить микрофон" else "Запустить радар")
                        }

                        Button(
                            onClick = {
                                audioController?.processSyntheticChirp()
                            },
                            colors = ButtonDefaults.buttonColors(
                                containerColor = MaterialTheme.colorScheme.secondary
                            ),
                            modifier = Modifier.weight(1f)
                        ) {
                            Text("Тест: Соловей")
                        }
                    }
                }
            }
        }

        // Live Spectrogram Image
        item {
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
                shape = RoundedCornerShape(12.dp)
            ) {
                Column(modifier = Modifier.padding(14.dp)) {
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically
                    ) {
                        Text(
                            text = "Mel-Спектрограмма (96 x 278):",
                            style = MaterialTheme.typography.titleSmall,
                            fontWeight = FontWeight.SemiBold
                        )
                        audioResult?.let { res ->
                            Text(
                                text = "DSP: ${"%.1f".format(res.dspMs)} ms | LiteRT: ${"%.1f".format(res.inferenceMs)} ms",
                                style = MaterialTheme.typography.labelSmall,
                                fontFamily = FontFamily.Monospace,
                                color = MaterialTheme.colorScheme.primary,
                                fontWeight = FontWeight.Bold
                            )
                        }
                    }

                    Spacer(modifier = Modifier.height(10.dp))

                    val bitmap = audioResult?.spectrogramBitmap
                    if (bitmap != null) {
                        Image(
                            bitmap = bitmap.asImageBitmap(),
                            contentDescription = "Live Mel Spectrogram",
                            modifier = Modifier
                                .fillMaxWidth()
                                .aspectRatio(278f / 96f)
                                .clip(RoundedCornerShape(8.dp)),
                            contentScale = ContentScale.FillWidth
                        )
                    } else {
                        Box(
                            modifier = Modifier
                                .fillMaxWidth()
                                .height(120.dp)
                                .background(Color.Black.copy(alpha = 0.05f), RoundedCornerShape(8.dp)),
                            contentAlignment = Alignment.Center
                        ) {
                            Text(
                                text = "Нажмите «Запустить радар» или «Тест: Соловей»",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant
                            )
                        }
                    }
                }
            }
        }

        // Identified Acoustic Signals
        item {
            Text(
                text = "Биоакустические сигналы:",
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Bold
            )
        }

        val acousticTaxa = audioResult?.topTaxa.orEmpty()
        if (acousticTaxa.isEmpty()) {
            item {
                Text(
                    text = "Ожидание звуковых сигналов...",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant
                )
            }
        } else {
            items(acousticTaxa) { taxon ->
                TaxonItemCard(taxon = taxon, onClick = { onTaxonSelected(taxon) })
            }
        }
    }
}

// ============================================================================
// SHARED UI COMPONENTS
// ============================================================================

@Composable
fun TaxonItemCard(taxon: TaxonSummary, onClick: () -> Unit) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
        shape = RoundedCornerShape(12.dp)
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = taxon.commonNameRu,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold
                )
                Text(
                    text = taxon.scientificName,
                    style = MaterialTheme.typography.bodyMedium,
                    fontFamily = FontFamily.Serif,
                    color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f)
                )
                Spacer(modifier = Modifier.height(6.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    KingdomBadge(taxon.kingdom)
                    SafetyBadge(taxon.safety)
                }
            }

            Column(horizontalAlignment = Alignment.End) {
                if (taxon.confidence > 0f) {
                    Text(
                        text = "%.1f%%".format(taxon.confidence * 100),
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                        color = MaterialTheme.colorScheme.primary
                    )
                }
                if (taxon.geoVerified) {
                    Surface(
                        color = Color(0xFFE8F5E9),
                        shape = RoundedCornerShape(4.dp)
                    ) {
                        Text(
                            text = "H3: подтверждено",
                            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                            style = MaterialTheme.typography.labelSmall,
                            color = Color(0xFF2E7D32),
                            fontWeight = FontWeight.Medium
                        )
                    }
                }
            }
        }
    }
}

@Composable
fun TaxonDetailDialog(detail: TaxonDetail, onDismiss: () -> Unit) {
    Dialog(onDismissRequest = onDismiss) {
        Card(
            modifier = Modifier
                .fillMaxWidth()
                .padding(8.dp),
            shape = RoundedCornerShape(16.dp),
            colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface)
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(20.dp)
            ) {
                Text(
                    text = detail.summary.commonNameRu,
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.Bold
                )
                Text(
                    text = detail.summary.scientificName,
                    style = MaterialTheme.typography.titleSmall,
                    fontFamily = FontFamily.Serif,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.7f)
                )

                Spacer(modifier = Modifier.height(12.dp))
                HorizontalDivider()
                Spacer(modifier = Modifier.height(12.dp))

                Text(
                    text = "Uber H3 ячейка (Res 4):",
                    style = MaterialTheme.typography.labelMedium,
                    fontWeight = FontWeight.SemiBold
                )
                Text(
                    text = detail.h3CellHex,
                    style = MaterialTheme.typography.bodyMedium,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.primary
                )

                Spacer(modifier = Modifier.height(12.dp))

                Text(
                    text = "Описание (распаковано из Zstd):",
                    style = MaterialTheme.typography.labelMedium,
                    fontWeight = FontWeight.SemiBold
                )
                Spacer(modifier = Modifier.height(4.dp))
                Surface(
                    color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f),
                    shape = RoundedCornerShape(8.dp),
                    modifier = Modifier.fillMaxWidth()
                ) {
                    Text(
                        text = detail.descriptionMarkdown,
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.padding(12.dp)
                    )
                }

                Spacer(modifier = Modifier.height(16.dp))
                Button(
                    onClick = onDismiss,
                    modifier = Modifier.align(Alignment.End)
                ) {
                    Text("Закрыть")
                }
            }
        }
    }
}

@Composable
fun KingdomBadge(kingdom: Kingdom) {
    val (label, bg, fg) = when (kingdom) {
        Kingdom.PLANTAE -> Triple("Флора", Color(0xFFE8F5E9), Color(0xFF2E7D32))
        Kingdom.FUNGI -> Triple("Грибы", Color(0xFFFFF3E0), Color(0xFFE65100))
        Kingdom.ANIMALIA -> Triple("Фауна", Color(0xFFE3F2FD), Color(0xFF1565C0))
    }
    Surface(color = bg, shape = RoundedCornerShape(4.dp)) {
        Text(
            text = label,
            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
            style = MaterialTheme.typography.labelSmall,
            color = fg,
            fontWeight = FontWeight.SemiBold
        )
    }
}

@Composable
fun SafetyBadge(safety: SafetyStatus) {
    val (label, bg, fg) = when (safety) {
        SafetyStatus.EDIBLE -> Triple("Съедобно", Color(0xFFE8F5E9), Color(0xFF2E7D32))
        SafetyStatus.TOXIC -> Triple("Ядовито", Color(0xFFFFEBEE), Color(0xFFC62828))
        SafetyStatus.DEADLY -> Triple("Смертельно", Color(0xFFB71C1C), Color.White)
        SafetyStatus.PROTECTED -> Triple("Красная Книга", Color(0xFFEDE7F6), Color(0xFF4A148C))
        SafetyStatus.NEUTRAL -> Triple("Нейтрально", Color(0xFFF5F5F5), Color(0xFF616161))
    }
    Surface(color = bg, shape = RoundedCornerShape(4.dp)) {
        Text(
            text = label,
            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
            style = MaterialTheme.typography.labelSmall,
            color = fg,
            fontWeight = FontWeight.SemiBold
        )
    }
}
