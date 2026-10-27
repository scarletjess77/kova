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
