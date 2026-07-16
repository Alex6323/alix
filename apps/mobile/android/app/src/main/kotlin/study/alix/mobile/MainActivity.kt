package study.alix.mobile

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.Settings
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    // The All-Files-Access dance plus small platform queries, kept as one
    // plain channel. Folder selection is NOT here: alix browses folders
    // in-app (it holds full filesystem access), which sidesteps the system
    // SAF picker whose DocumentsUI crashes on some devices. The plugin
    // ecosystem for these is mid-migration to AGP 9 / built-in Kotlin and does
    // not build against this project's toolchain; a few small calls do not
    // earn a dependency anyway.
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "alix/platform")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "sdkInt" -> result.success(Build.VERSION.SDK_INT)
                    "appVersion" -> {
                        val info = packageManager.getPackageInfo(packageName, 0)
                        result.success("${info.versionName}+${info.longVersionCode}")
                    }
                    "hasAllFilesAccess" -> result.success(
                        Build.VERSION.SDK_INT >= 30 && Environment.isExternalStorageManager()
                    )
                    "requestAllFilesAccess" -> {
                        // The app-scoped settings page; some OEM builds only
                        // ship the global one, so fall back to that.
                        try {
                            startActivity(
                                Intent(
                                    Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                                    Uri.parse("package:$packageName"),
                                )
                            )
                        } catch (e: Exception) {
                            startActivity(Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION))
                        }
                        result.success(null)
                    }
                    else -> result.notImplemented()
                }
            }
    }
}
