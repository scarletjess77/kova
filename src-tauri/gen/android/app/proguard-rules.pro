# Kova's own classes are few and reached by name: the plugin Tauri loads by its class
# name, the VPN, tile and widget services from the manifest, and the arguments Jackson
# fills in. Keep them whole; shrinking still applies to the libraries.
-keep class app.kova.client.** { *; }

# Jackson reads Kotlin classes through their metadata and annotations.
-keep class kotlin.Metadata { *; }
-keepattributes *Annotation*, Signature, InnerClasses, EnclosingMethod, RuntimeVisibleAnnotations, RuntimeVisibleParameterAnnotations
-keepnames class com.fasterxml.jackson.** { *; }
-dontwarn com.fasterxml.jackson.databind.**

# Line numbers in crash reports.
-keepattributes SourceFile, LineNumberTable

# The QR scanner (ML Kit) creates its components by reflection, from class names in the
# manifest; R8's full mode would drop their constructors and the camera screen crashes.
-keep class * implements com.google.firebase.components.ComponentRegistrar { *; }
-keep class com.google.mlkit.** { *; }
