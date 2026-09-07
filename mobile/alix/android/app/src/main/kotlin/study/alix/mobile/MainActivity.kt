package study.alix.mobile

import android.os.Build
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
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
                    else -> result.notImplemented()
                }
            }
    }
}
