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
    private var pendingPick: MethodChannel.Result? = null

    // The shared-decks-folder plumbing: the All-Files-Access dance plus the
    // system folder picker, kept as one plain channel. The plugin ecosystem
    // for these is mid-migration to AGP 9 / built-in Kotlin and does not
    // build against this project's toolchain; four small calls do not earn
    // a dependency anyway.
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "alix/platform")
            .setMethodCallHandler { call, result ->
                when (call.method) {
                    "sdkInt" -> result.success(Build.VERSION.SDK_INT)
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
                    "pickDirectory" -> {
                        if (pendingPick != null) {
                            result.error("busy", "a folder pick is already open", null)
                        } else {
                            pendingPick = result
                            startActivityForResult(
                                Intent(Intent.ACTION_OPEN_DOCUMENT_TREE),
                                PICK_DIRECTORY,
                            )
                        }
                    }
                    else -> result.notImplemented()
                }
            }
    }

    // Returns the picked tree URI as a string (Dart maps it to a real path);
    // null on cancel.
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        if (requestCode == PICK_DIRECTORY) {
            val uri = if (resultCode == RESULT_OK) data?.data?.toString() else null
            pendingPick?.success(uri)
            pendingPick = null
            return
        }
        super.onActivityResult(requestCode, resultCode, data)
    }

    private companion object {
        const val PICK_DIRECTORY = 41337
    }
}
