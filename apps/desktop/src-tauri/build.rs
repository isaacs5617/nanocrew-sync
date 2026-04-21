fn main() {
    // Delay-load WinFsp's DLL so the app can still launch on machines without
    // WinFsp installed (the binary links but resolves the DLL lazily). The
    // Settings > Advanced > WinFsp panel surfaces the missing-install state.
    winfsp_build::build();
    tauri_build::build()
}
