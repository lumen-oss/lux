/// The extension for C shared libraries.
pub(crate) fn c_dylib_extension() -> &'static str {
    if cfg!(target_env = "msvc") {
        "dll"
    } else {
        "so"
    }
}

/// The extension for C static libraries.
pub(crate) fn c_lib_extension() -> &'static str {
    if cfg!(target_env = "msvc") {
        "lib"
    } else {
        "a"
    }
}

/// The extension for C objects.
pub(crate) fn c_obj_extension() -> &'static str {
    if cfg!(target_env = "msvc") {
        "obj"
    } else {
        "o"
    }
}

pub(crate) fn default_cflags() -> &'static str {
    if cfg!(target_env = "msvc") {
        "/NOLOGO /MD /O2"
    } else {
        "-O2"
    }
}

pub(crate) fn default_libflag() -> &'static str {
    if cfg!(target_os = "macos") {
        "-bundle -undefined dynamic_lookup -all_load"
    } else if cfg!(target_env = "msvc") {
        "/NOLOGO /DLL"
    } else {
        "-shared"
    }
}
