package com.example.folialens

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.example.folialens.core.NativeBridge
import com.example.folialens.core.uniffi.rustHealthCheck
import com.example.folialens.ui.theme.FoliaLensTheme

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        setContent {
            FoliaLensTheme {
                Scaffold(modifier = Modifier.fillMaxSize()) { innerPadding ->
                    RustBridgeHealthScreen(modifier = Modifier.padding(innerPadding))
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
fun RustBridgeHealthScreen(modifier: Modifier = Modifier) {
    val (jniResult, jniSuccess, uniffiResult, uniffiSuccess) = remember {
        val (jniRes, jniOk) = try {
            NativeBridge.jniHealthCheck() to true
        } catch (t: Throwable) {
            "JNI Error: ${t.message ?: t.javaClass.simpleName}" to false
        }

        val (uniffiRes, uniffiOk) = try {
            rustHealthCheck() to true
        } catch (t: Throwable) {
            "UniFFI Error: ${t.message ?: t.javaClass.simpleName}" to false
        }

        BridgeStatus(
            jniResult = jniRes,
            jniSuccess = jniOk,
            uniffiResult = uniffiRes,
            uniffiSuccess = uniffiOk,
        )
    }

    val allOperational = jniSuccess && uniffiSuccess

    Box(
        modifier = modifier
            .fillMaxSize()
            .padding(24.dp),
        contentAlignment = Alignment.Center,
    ) {
        Card(
            modifier = Modifier.fillMaxWidth(),
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceVariant,
            ),
            elevation = CardDefaults.cardElevation(defaultElevation = 6.dp),
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                // Status Header Badge
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.Center,
                ) {
                    Box(
                        modifier = Modifier
                            .size(14.dp)
                            .background(
                                color = if (allOperational) Color(0xFF2E7D32) else Color(0xFFC62828),
                                shape = CircleShape,
                            ),
                    )
                    Spacer(modifier = Modifier.width(10.dp))
                    Text(
                        text = if (allOperational) "CORE STATUS: ACTIVE" else "CORE STATUS: ERROR",
                        style = MaterialTheme.typography.labelLarge,
                        color = if (allOperational) Color(0xFF2E7D32) else Color(0xFFC62828),
                        fontWeight = FontWeight.Bold,
                        letterSpacing = 1.2.sp,
                    )
                }

                Spacer(modifier = Modifier.height(16.dp))

                Text(
                    text = "FoliaLens Hybrid Bridge",
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.Bold,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )

                Spacer(modifier = Modifier.height(8.dp))

                Text(
                    text = "Kotlin + Rust NDK (UniFFI & Direct JNI)",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.7f),
                )

                Spacer(modifier = Modifier.height(20.dp))
                HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                Spacer(modifier = Modifier.height(20.dp))

                // Direct JNI section
                StatusSection(
                    title = "1. Direct JNI Check",
                    message = jniResult,
                    isSuccess = jniSuccess,
                )

                Spacer(modifier = Modifier.height(16.dp))

                // UniFFI section
                StatusSection(
                    title = "2. UniFFI Export Check",
                    message = uniffiResult,
                    isSuccess = uniffiSuccess,
                )
            }
        }
    }
}

@Composable
fun StatusSection(title: String, message: String, isSuccess: Boolean) {
    Column(
        modifier = Modifier.fillMaxWidth(),
        horizontalAlignment = Alignment.Start,
    ) {
        Text(
            text = title,
            style = MaterialTheme.typography.titleSmall,
            fontWeight = FontWeight.SemiBold,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(modifier = Modifier.height(6.dp))
        Surface(
            modifier = Modifier.fillMaxWidth(),
            shape = MaterialTheme.shapes.small,
            color = if (isSuccess) Color(0xFFE8F5E9) else Color(0xFFFFEBEE),
        ) {
            Text(
                text = message,
                modifier = Modifier.padding(12.dp),
                style = MaterialTheme.typography.bodyMedium,
                fontFamily = FontFamily.Monospace,
                color = if (isSuccess) Color(0xFF1B5E20) else Color(0xFFB71C1C),
                fontWeight = FontWeight.Medium,
            )
        }
    }
}

@Preview(showBackground = true)
@Composable
fun RustBridgeHealthScreenPreview() {
    FoliaLensTheme {
        RustBridgeHealthScreen()
    }
}
