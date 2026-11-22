// COM finds DllGetClassObject and DllCanUnloadNow with GetProcAddress, so the
// linker wants them exported PRIVATE: in the DLL's export table but not in its
// import library (LNK4104 otherwise). rustc's .def file cannot say that; an
// /EXPORT option with PRIVATE for the same name overrides its entry.
fn main() {
    for name in ["DllGetClassObject", "DllCanUnloadNow"] {
        println!("cargo:rustc-cdylib-link-arg=/EXPORT:{name},PRIVATE");
    }
}
