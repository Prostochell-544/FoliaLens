package com.example.folialens

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
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
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import com.example.folialens.core.NativeBridge
import com.example.folialens.core.uniffi.AtlasEngine
import com.example.folialens.core.uniffi.Kingdom
import com.example.folialens.core.uniffi.SafetyStatus
import com.example.folialens.core.uniffi.TaxonDetail
import com.example.folialens.core.uniffi.TaxonSummary
import com.example.folialens.core.uniffi.rustHealthCheck
import com.example.folialens.ui.theme.FoliaLensTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            FoliaLensTheme {
                Scaffold(modifier = Modifier.fillMaxSize()) { innerPadding ->
                    AtlasMainScreen(modifier = Modifier.padding(innerPadding))
                }
            }
        }
    }
}

data class BridgeStatus(
    val jniResult: String,
    val jniSuccess: Boolean,
    val uniffiResult: String,
    val uniffiSuccess: Boolean,
)

@Composable
fun AtlasMainScreen(modifier: Modifier = Modifier) {
    // 1. Core health checks
    val (jniResult, jniSuccess, uniffiResult, uniffiSuccess) = remember {
        var jniRes: String
        var jniOk: Boolean
        try {
            jniRes = NativeBridge.jniHealthCheck()
            jniOk = true
        } catch (t: Throwable) {
            jniRes = "JNI Error: ${t.message ?: t.javaClass.simpleName}"
            jniOk = false
        }

        var uniffiRes: String
        var uniffiOk: Boolean
        try {
            uniffiRes = rustHealthCheck()
            uniffiOk = true
        } catch (t: Throwable) {
            uniffiRes = "UniFFI Error: ${t.message ?: t.javaClass.simpleName}"
            uniffiOk = false
        }

        BridgeStatus(
            jniResult = jniRes,
            jniSuccess = jniOk,
            uniffiResult = uniffiRes,
            uniffiSuccess = uniffiOk,
        )
    }

    // 2. Atlas Engine instance
    var engineError by remember { mutableStateOf<String?>(null) }
    val engine = remember {
        try {
            AtlasEngine.newInMemoryDemo()
        } catch (t: Throwable) {
            engineError = "Failed to init AtlasEngine: ${t.message}"
            null
        }
    }

    // 3. UI States
    var searchQuery by remember { mutableStateOf("") }
    var searchResults by remember { mutableStateOf<List<TaxonSummary>>(emptyList()) }
    var selectedTaxonDetail by remember { mutableStateOf<TaxonDetail?>(null) }
    var inferenceResults by remember { mutableStateOf<List<TaxonSummary>?>(null) }

    // Initial search load
    LaunchedEffect(engine, searchQuery) {
        if (engine != null) {
            try {
                searchResults = engine.searchTaxa(searchQuery, 10u)
            } catch (_: Throwable) {
                // Keep current results if query syntax error
            }
        }
    }

    LazyColumn(
        modifier = modifier
            .fillMaxSize()
            .padding(horizontal = 16.dp),
        contentPadding = PaddingValues(vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        // --- HEADER & BRIDGE STATUS ---
        item {
            HeaderStatusCard(
                jniResult = jniResult,
                jniSuccess = jniSuccess,
                uniffiResult = uniffiResult,
                uniffiSuccess = uniffiSuccess,
                engineReady = engine != null,
                engineError = engineError,
            )
        }

        // --- SECTION 1: SEARCH DATABASE ---
        item {
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
                elevation = CardDefaults.cardElevation(defaultElevation = 2.dp),
            ) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Text(
                        text = "Полнотекстовый поиск (SQLite FTS5)",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                    )
                    Spacer(modifier = Modifier.height(8.dp))
                    OutlinedTextField(
                        value = searchQuery,
                        onValueChange = { searchQuery = it },
                        modifier = Modifier.fillMaxWidth(),
                        placeholder = { Text("Поиск: Мухомор, Береза, Лисичка...") },
                        singleLine = true,
                    )
                }
            }
        }

        // --- SECTION 2: VISION INFERENCE & GEO PENALTY SIMULATION ---
        item {
            Card(
                modifier = Modifier.fillMaxWidth(),
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.secondaryContainer,
                ),
                elevation = CardDefaults.cardElevation(defaultElevation = 2.dp),
            ) {
                Column(modifier = Modifier.padding(16.dp)) {
                    Text(
                        text = "Симуляция Vision + H3 Гео-фильтр",
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.Bold,
                        color = MaterialTheme.colorScheme.onSecondaryContainer,
                    )
                    Spacer(modifier = Modifier.height(4.dp))
                    Text(
                        text = "Логиты модели: [0.8, 2.1, 0.5, 0.1, 0.3]\nGPS: 59.93, 30.33 (СПб, Июль), штраф -0.15 за несезон/регион",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSecondaryContainer.copy(alpha = 0.8f),
                    )
                    Spacer(modifier = Modifier.height(12.dp))
                    Button(
                        onClick = {
                            if (engine != null) {
                                val testLogits = listOf(0.8f, 2.1f, 0.5f, 0.1f, 0.3f)
                                inferenceResults = engine.postprocessVisionLogits(
                                    logits = testLogits,
                                    lat = 59.93,
                                    lon = 30.33,
                                    month = 7u.toUByte(),
                                    topK = 5u,
                                )
                            }
                        },
                        colors = ButtonDefaults.buttonColors(
                            containerColor = MaterialTheme.colorScheme.primary,
                        ),
                    ) {
                        Text("Запустить Softmax + H3 постпроцессинг")
                    }
                }
            }
        }

        // --- INFERENCE RESULTS ---
        if (inferenceResults != null) {
            item {
                Text(
                    text = "Результаты постпроцессинга с гео-валидацией:",
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.Bold,
                )
            }
            items(inferenceResults.orEmpty()) { taxon ->
                TaxonItemCard(
                    taxon = taxon,
                    onClick = {
                        engine?.let { selectedTaxonDetail = it.getTaxonDetail(taxon.taxonId) }
                    },
                )
            }
        }

        // --- SEARCH RESULTS LIST ---
        item {
            Text(
                text = "Таксоны базы данных (${searchResults.size}):",
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.Bold,
            )
        }
        items(searchResults) { taxon ->
            TaxonItemCard(
                taxon = taxon,
                onClick = {
                    engine?.let { selectedTaxonDetail = it.getTaxonDetail(taxon.taxonId) }
                },
            )
        }
    }

    // --- TAXON DETAIL MODAL (ZSTD UNCOMPRESSED) ---
    selectedTaxonDetail?.let { detail ->
        TaxonDetailDialog(
            detail = detail,
            onDismiss = { selectedTaxonDetail = null },
        )
    }
}

@Composable
fun TaxonItemCard(taxon: TaxonSummary, onClick: () -> Unit) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
        shape = RoundedCornerShape(12.dp),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = taxon.commonNameRu,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold,
                )
                Text(
                    text = taxon.scientificName,
                    style = MaterialTheme.typography.bodyMedium,
                    fontFamily = FontFamily.Serif,
                    color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f),
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
                        color = MaterialTheme.colorScheme.primary,
                    )
                }
                if (taxon.geoVerified) {
                    Surface(
                        color = Color(0xFFE8F5E9),
                        shape = RoundedCornerShape(4.dp),
                    ) {
                        Text(
                            text = "H3: подтверждено",
                            modifier = Modifier.padding(horizontal = 6.dp, vertical = 2.dp),
                            style = MaterialTheme.typography.labelSmall,
                            color = Color(0xFF2E7D32),
                            fontWeight = FontWeight.Medium,
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
            colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(20.dp),
            ) {
                Text(
                    text = detail.summary.commonNameRu,
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.Bold,
                )
                Text(
                    text = detail.summary.scientificName,
                    style = MaterialTheme.typography.titleSmall,
                    fontFamily = FontFamily.Serif,
                    color = MaterialTheme.colorScheme.onSurface.copy(alpha = 0.7f),
                )

                Spacer(modifier = Modifier.height(12.dp))
                HorizontalDivider()
                Spacer(modifier = Modifier.height(12.dp))

                Text(
                    text = "Uber H3 ячейка (Res 4):",
                    style = MaterialTheme.typography.labelMedium,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    text = detail.h3CellHex,
                    style = MaterialTheme.typography.bodyMedium,
                    fontFamily = FontFamily.Monospace,
                    color = MaterialTheme.colorScheme.primary,
                )

                Spacer(modifier = Modifier.height(12.dp))

                Text(
                    text = "Описание (распаковано из Zstd):",
                    style = MaterialTheme.typography.labelMedium,
                    fontWeight = FontWeight.SemiBold,
                )
                Spacer(modifier = Modifier.height(4.dp))
                Surface(
                    color = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f),
                    shape = RoundedCornerShape(8.dp),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Text(
                        text = detail.descriptionMarkdown,
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.padding(12.dp),
                    )
                }

                Spacer(modifier = Modifier.height(16.dp))
                Button(
                    onClick = onDismiss,
                    modifier = Modifier.align(Alignment.End),
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
            fontWeight = FontWeight.SemiBold,
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
            fontWeight = FontWeight.SemiBold,
        )
    }
}

@Composable
fun HeaderStatusCard(
    jniResult: String,
    jniSuccess: Boolean,
    uniffiResult: String,
    uniffiSuccess: Boolean,
    engineReady: Boolean,
    engineError: String?,
) {
    val allOperational = jniSuccess && uniffiSuccess && engineReady

    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
        elevation = CardDefaults.cardElevation(defaultElevation = 4.dp),
    ) {
        Column(modifier = Modifier.padding(16.dp)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Text(
                    text = "FoliaLens Core NDK",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Bold,
                )
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Box(
                        modifier = Modifier
                            .size(10.dp)
                            .background(
                                color = if (allOperational) Color(0xFF2E7D32) else Color(0xFFC62828),
                                shape = CircleShape,
                            ),
                    )
                    Spacer(modifier = Modifier.width(6.dp))
                    Text(
                        text = if (allOperational) "ACTIVE" else "ERROR",
                        style = MaterialTheme.typography.labelSmall,
                        color = if (allOperational) Color(0xFF2E7D32) else Color(0xFFC62828),
                        fontWeight = FontWeight.Bold,
                    )
                }
            }

            Spacer(modifier = Modifier.height(8.dp))
            Text(
                text = "Direct JNI: $jniResult | UniFFI: $uniffiResult",
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
            )
            if (engineError != null) {
                Text(
                    text = engineError,
                    style = MaterialTheme.typography.bodySmall,
                    color = Color.Red,
                )
            }
        }
    }
}

@Preview(showBackground = true)
@Composable
fun AtlasMainScreenPreview() {
    FoliaLensTheme {
        AtlasMainScreen()
    }
}
