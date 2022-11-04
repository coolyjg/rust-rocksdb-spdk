use std::collections::HashSet;
use std::path::Path;
use std::{env, fs, path::PathBuf, process::Command};

fn link(name: &str, bundled: bool) {
    use std::env::var;
    let target = var("TARGET").unwrap();
    let target: Vec<_> = target.split('-').collect();
    if target.get(2) == Some(&"windows") {
        println!("cargo:rustc-link-lib=dylib={}", name);
        if bundled && target.get(3) == Some(&"gnu") {
            let dir = var("CARGO_MANIFEST_DIR").unwrap();
            println!("cargo:rustc-link-search=native={}/{}", dir, target[0]);
        }
    }
}

fn fail_on_empty_directory(name: &str) {
    if fs::read_dir(name).unwrap().count() == 0 {
        println!(
            "The `{}` directory is empty, did you forget to pull the submodules?",
            name
        );
        println!("Try `git submodule update --init --recursive`");
        panic!();
    }
}

fn rocksdb_include_dir() -> String {
    match env::var("ROCKSDB_INCLUDE_DIR") {
        Ok(val) => val,
        Err(_) => "rocksdb/include".to_string(),
    }
}

fn bindgen_rocksdb() {
    #[cfg(feature = "spdk")]
    let src = env::current_dir().unwrap().join("spdk");
    #[cfg(feature = "spdk")]
    let ignored_macros = IgnoreMacros(
        vec![
            "FP_INFINITE".into(),
            "FP_NAN".into(),
            "FP_NORMAL".into(),
            "FP_SUBNORMAL".into(),
            "FP_ZERO".into(),
            // "IPPORT_RESERVED".into(),
        ]
        .into_iter()
        .collect(),
    );

    #[cfg(not(feature = "spdk"))]
    let bindings = bindgen::Builder::default()
        .header(rocksdb_include_dir() + "/rocksdb/c.h")
        .derive_debug(false)
        .blocklist_type("max_align_t")
        .ctypes_prefix("libc")
        .size_t_is_usize(true)
        .generate()
        .expect("unable to generate rocksdb bindings");

    #[cfg(feature = "spdk")]
    let bindings = bindgen::Builder::default()
        .clang_arg(format!("-I{}", src.join("build/include").display()))
        .header(rocksdb_include_dir() + "/rocksdb/c.h")
        .header("wrapper.h")
        .parse_callbacks(Box::new(ignored_macros))
        .derive_debug(false)
        .blocklist_item("IPPORT_.*")
        // XXX: workaround for 'error[E0588]: packed type cannot transitively contain a `#[repr(align)]` type'
        .blocklist_type("spdk_nvme_tcp_rsp")
        .blocklist_type("spdk_nvme_tcp_cmd")
        .blocklist_type("spdk_nvmf_fabric_prop_get_rsp")
        .blocklist_type("spdk_nvmf_fabric_connect_rsp")
        .blocklist_type("spdk_nvmf_fabric_connect_cmd")
        .blocklist_type("spdk_nvmf_fabric_auth_send_cmd")
        .blocklist_type("spdk_nvmf_fabric_auth_recv_cmd")
        .blocklist_type("spdk_nvme_health_information_page")
        .blocklist_type("spdk_nvme_ctrlr_data")
        .blocklist_function("spdk_nvme_ctrlr_get_data")
        .opaque_type("spdk_nvme_sgl_descriptor")
        .blocklist_type("max_align_t") // https://github.com/rust-lang-nursery/rust-bindgen/issues/550
        .ctypes_prefix("libc")
        .size_t_is_usize(true)
        .generate()
        .expect("unable to generate rocksdb bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("unable to write rocksdb bindings");
}

fn build_rocksdb() {
    let target = env::var("TARGET").unwrap();

    let mut config = cc::Build::new();
    config.include("rocksdb/include/");
    config.include("rocksdb/");
    config.include("rocksdb/third-party/gtest-1.8.1/fused-src/");
    #[cfg(feature = "spdk")]
    config.include("spdk/build/include");

    if cfg!(feature = "snappy") {
        config.define("SNAPPY", Some("1"));
        config.include("snappy/");
    }

    if cfg!(feature = "lz4") {
        config.define("LZ4", Some("1"));
        if let Some(path) = env::var_os("DEP_LZ4_INCLUDE") {
            config.include(path);
        }
    }

    if cfg!(feature = "zstd") {
        config.define("ZSTD", Some("1"));
        if let Some(path) = env::var_os("DEP_ZSTD_INCLUDE") {
            config.include(path);
        }
    }

    if cfg!(feature = "zlib") {
        config.define("ZLIB", Some("1"));
        if let Some(path) = env::var_os("DEP_Z_INCLUDE") {
            config.include(path);
        }
    }

    if cfg!(feature = "bzip2") {
        config.define("BZIP2", Some("1"));
        if let Some(path) = env::var_os("DEP_BZIP2_INCLUDE") {
            config.include(path);
        }
    }

    if cfg!(feature = "rtti") {
        config.define("USE_RTTI", Some("1"));
    }

    config.include(".");
    config.define("NDEBUG", Some("1"));

    #[cfg(not(feature = "spdk"))]
    let mut lib_sources = include_str!("rocksdb_lib_sources.txt")
        .trim()
        .split('\n')
        .map(str::trim)
        // We have a pre-generated a version of build_version.cc in the local directory
        .filter(|file| !matches!(*file, "util/build_version.cc"))
        .collect::<Vec<&'static str>>();

    #[cfg(feature = "spdk")]
    let mut lib_sources = include_str!("rocksdb_lib_sources_spdk.txt")
        .trim()
        .split('\n')
        .map(str::trim)
        // We have a pre-generated a version of build_version.cc in the local directory
        .filter(|file| !matches!(*file, "util/build_version.cc"))
        .collect::<Vec<&'static str>>();

    if target.contains("x86_64") {
        // This is needed to enable hardware CRC32C. Technically, SSE 4.2 is
        // only available since Intel Nehalem (about 2010) and AMD Bulldozer
        // (about 2011).
        let target_feature = env::var("CARGO_CFG_TARGET_FEATURE").unwrap();
        let target_features: Vec<_> = target_feature.split(',').collect();
        if target_features.contains(&"sse2") {
            config.flag_if_supported("-msse2");
        }
        if target_features.contains(&"sse4.1") {
            config.flag_if_supported("-msse4.1");
        }
        if target_features.contains(&"sse4.2") {
            config.flag_if_supported("-msse4.2");
            config.define("HAVE_SSE42", Some("1"));
        }
        // Pass along additional target features as defined in
        // build_tools/build_detect_platform.
        if target_features.contains(&"avx2") {
            config.flag_if_supported("-mavx2");
            config.define("HAVE_AVX2", Some("1"));
        }
        if target_features.contains(&"bmi1") {
            config.flag_if_supported("-mbmi");
            config.define("HAVE_BMI", Some("1"));
        }
        if target_features.contains(&"lzcnt") {
            config.flag_if_supported("-mlzcnt");
            config.define("HAVE_LZCNT", Some("1"));
        }
        if !target.contains("android") && target_features.contains(&"pclmulqdq") {
            config.define("HAVE_PCLMUL", Some("1"));
            config.flag_if_supported("-mpclmul");
        }
    }

    if target.contains("apple-ios") {
        config.define("OS_MACOSX", None);

        config.define("IOS_CROSS_COMPILE", None);
        config.define("PLATFORM", "IOS");
        config.define("NIOSTATS_CONTEXT", None);
        config.define("NPERF_CONTEXT", None);
        config.define("ROCKSDB_PLATFORM_POSIX", None);
        config.define("ROCKSDB_LIB_IO_POSIX", None);

        env::set_var("IPHONEOS_DEPLOYMENT_TARGET", "11.0");
    } else if target.contains("darwin") {
        config.define("OS_MACOSX", None);
        config.define("ROCKSDB_PLATFORM_POSIX", None);
        config.define("ROCKSDB_LIB_IO_POSIX", None);
    } else if target.contains("android") {
        config.define("OS_ANDROID", None);
        config.define("ROCKSDB_PLATFORM_POSIX", None);
        config.define("ROCKSDB_LIB_IO_POSIX", None);
    } else if target.contains("linux") {
        config.define("OS_LINUX", None);
        config.define("ROCKSDB_PLATFORM_POSIX", None);
        config.define("ROCKSDB_LIB_IO_POSIX", None);
    } else if target.contains("freebsd") {
        config.define("OS_FREEBSD", None);
        config.define("ROCKSDB_PLATFORM_POSIX", None);
        config.define("ROCKSDB_LIB_IO_POSIX", None);
    } else if target.contains("windows") {
        link("rpcrt4", false);
        link("shlwapi", false);
        config.define("DWIN32", None);
        config.define("OS_WIN", None);
        config.define("_MBCS", None);
        config.define("WIN64", None);
        config.define("NOMINMAX", None);
        config.define("ROCKSDB_WINDOWS_UTF8_FILENAMES", None);

        if &target == "x86_64-pc-windows-gnu" {
            // Tell MinGW to create localtime_r wrapper of localtime_s function.
            config.define("_POSIX_C_SOURCE", Some("1"));
            // Tell MinGW to use at least Windows Vista headers instead of the ones of Windows XP.
            // (This is minimum supported version of rocksdb)
            config.define("_WIN32_WINNT", Some("_WIN32_WINNT_VISTA"));
        }

        // Remove POSIX-specific sources
        lib_sources = lib_sources
            .iter()
            .cloned()
            .filter(|file| {
                !matches!(
                    *file,
                    "port/port_posix.cc"
                        | "env/env_posix.cc"
                        | "env/fs_posix.cc"
                        | "env/io_posix.cc"
                )
            })
            .collect::<Vec<&'static str>>();

        // Add Windows-specific sources
        lib_sources.extend([
            "port/win/env_default.cc",
            "port/win/port_win.cc",
            "port/win/xpress_win.cc",
            "port/win/io_win.cc",
            "port/win/win_thread.cc",
            "port/win/env_win.cc",
            "port/win/win_logger.cc",
        ]);

        if cfg!(feature = "jemalloc") {
            lib_sources.push("port/win/win_jemalloc.cc");
        }
    }

    config.define("ROCKSDB_SUPPORT_THREAD_LOCAL", None);

    if cfg!(feature = "jemalloc") {
        config.define("WITH_JEMALLOC", "ON");
    }

    #[cfg(feature = "io-uring")]
    if target.contains("linux") {
        pkg_config::probe_library("liburing")
            .expect("The io-uring feature was requested but the library is not available");
        config.define("ROCKSDB_IOURING_PRESENT", Some("1"));
    }

    if target.contains("msvc") {
        config.flag("-EHsc");
        config.flag("-std:c++17");
    } else {
        config.flag(&cxx_standard());
        // matches the flags in CMakeLists.txt from rocksdb
        config.define("HAVE_UINT128_EXTENSION", Some("1"));
        config.flag("-Wsign-compare");
        config.flag("-Wshadow");
        config.flag("-Wno-unused-parameter");
        config.flag("-Wno-unused-variable");
        config.flag("-Woverloaded-virtual");
        config.flag("-Wnon-virtual-dtor");
        config.flag("-Wno-missing-field-initializers");
        config.flag("-Wno-strict-aliasing");
        config.flag("-Wno-invalid-offsetof");
    }

    for file in lib_sources {
        config.file(&format!("rocksdb/{file}"));
    }

    config.file("build_version.cc");

    config.cpp(true);
    config.flag_if_supported("-std=c++17");
    config.compile("librocksdb.a");
}

fn build_snappy() {
    let target = env::var("TARGET").unwrap();
    let endianness = env::var("CARGO_CFG_TARGET_ENDIAN").unwrap();
    let mut config = cc::Build::new();

    config.include("snappy/");
    config.include(".");
    config.define("NDEBUG", Some("1"));
    config.extra_warnings(false);

    if target.contains("msvc") {
        config.flag("-EHsc");
    } else {
        // Snappy requires C++11.
        // See: https://github.com/google/snappy/blob/master/CMakeLists.txt#L32-L38
        config.flag("-std=c++11");
    }

    if endianness == "big" {
        config.define("SNAPPY_IS_BIG_ENDIAN", Some("1"));
    }

    config.file("snappy/snappy.cc");
    config.file("snappy/snappy-sinksource.cc");
    config.file("snappy/snappy-c.cc");
    config.cpp(true);
    config.compile("libsnappy.a");
}

fn try_to_find_and_link_lib(lib_name: &str) -> bool {
    if let Ok(v) = env::var(&format!("{}_COMPILE", lib_name)) {
        if v.to_lowercase() == "true" || v == "1" {
            return false;
        }
    }

    if let Ok(lib_dir) = env::var(&format!("{}_LIB_DIR", lib_name)) {
        println!("cargo:rustc-link-search=native={}", lib_dir);
        let mode = match env::var_os(&format!("{}_STATIC", lib_name)) {
            Some(_) => "static",
            None => "dylib",
        };
        println!("cargo:rustc-link-lib={}={}", mode, lib_name.to_lowercase());
        return true;
    }
    false
}

fn cxx_standard() -> String {
    env::var("ROCKSDB_CXX_STD").map_or("-std=c++17".to_owned(), |cxx_std| {
        if !cxx_std.starts_with("-std=") {
            format!("-std={}", cxx_std)
        } else {
            cxx_std
        }
    })
}

fn update_submodules() {
    let program = "git";
    let dir = "../";
    let args = ["submodule", "update", "--init"];
    println!(
        "Running command: \"{} {}\" in dir: {}",
        program,
        args.join(" "),
        dir
    );
    let ret = Command::new(program).current_dir(dir).args(args).status();

    match ret.map(|status| (status.success(), status.code())) {
        Ok((true, _)) => (),
        Ok((false, Some(c))) => panic!("Command failed with error code {}", c),
        Ok((false, None)) => panic!("Command got killed"),
        Err(e) => panic!("Command failed with error: {}", e),
    }
}

#[cfg(feature = "spdk")]
fn build_spdk() {
    let src = env::current_dir().unwrap().join("spdk");
    let dst = PathBuf::from(env::var("OUT_DIR").unwrap()).join("libspdk_fat.so");

    // if dst.exists() {
    //     return;
    // }

    // update submodule
    if !Path::new("spdk/.git").exists() {
        let _ = Command::new("git")
            .args(&["submodule", "update", "--init", "--recursive"])
            .status();
    }

    // ./configure --without-isal
    let status = Command::new("bash")
        .current_dir(&src)
        .arg("./configure")
        .arg("--without-isal")
        .status()
        .expect("failed to configure");
    assert!(status.success(), "failed to configure: {}", status);

    // make
    let status = Command::new("make")
        .current_dir(&src)
        .arg(&format!("-j{}", env::var("NUM_JOBS").unwrap()))
        .status()
        .expect("failed to make");
    assert!(status.success(), "failed to make: {}", status);

    // link all shared libraries to generate libspdk_fat.so
    let mut cc = Command::new("cc");
    cc.arg("-shared")
        .arg("-o")
        .arg(dst.clone())
        .arg("-laio")
        .arg("-lnuma")
        .arg("-luuid")
        .arg("-lcrypto")
        .arg("-Wl,--whole-archive");

    let spdks = std::fs::read_dir(src.join("build/lib")).unwrap();
    let dpdks = std::fs::read_dir(src.join("dpdk/build/lib")).unwrap();
    for e in spdks.chain(dpdks) {
        let entry = e.expect("failed to read directory entry");
        let name = entry.file_name();
        let name = name.to_str().unwrap();
        if name == "libspdk_ut_mock.a" {
            continue;
        }
        if name.starts_with("lib") && name.ends_with(".a") {
            cc.arg(entry.path());
        }
    }
    cc.arg("-Wl,--no-whole-archive");
    let status = cc.status().expect("failed to generate libspdk_fat.so");
    assert!(
        status.success(),
        "failed to generate libspdk_fat.so: {}",
        status
    );

    let cp_dst = PathBuf::from(env::var("OUT_DIR").unwrap()).join("../../../libspdk_fat.so");
    let status = Command::new("cp")
        .arg(dst.clone())
        .arg(cp_dst)
        .status()
        .expect("failed to cp");
    assert!(status.success(), "failed to cp: {}", status);
}

fn main() {
    #[cfg(feature = "spdk")]
    build_spdk();

    #[cfg(feature = "spdk")]
    println!("cargo:rustc-link-lib=spdk_fat");
    #[cfg(feature = "spdk")]
    println!("cargo:rustc-link-lib=aio");
    #[cfg(feature = "spdk")]
    println!("cargo:rustc-link-lib=numa");
    #[cfg(feature = "spdk")]
    println!("cargo:rustc-link-lib=uuid");
    #[cfg(feature = "spdk")]
    println!("cargo:rustc-link-lib=crypto");
    #[cfg(feature = "spdk")]
    println!("cargo:rustc-link-lib=stdc++");
    #[cfg(feature = "spdk")]
    println!("cargo:rustc-link-lib=ssl");
    #[cfg(feature = "spdk")]
    println!(
        "cargo:rustc-link-search=native={}",
        env::var("OUT_DIR").unwrap()
    );

    // Tell cargo to invalidate the built crate whenever the wrapper changes
    #[cfg(feature = "spdk")]
    println!("cargo:rerun-if-changed=wrapper.h");

    #[cfg(feature = "spdk")]
    let ignored_macros = IgnoreMacros(
        vec![
            "FP_INFINITE".into(),
            "FP_NAN".into(),
            "FP_NORMAL".into(),
            "FP_SUBNORMAL".into(),
            "FP_ZERO".into(),
            // "IPPORT_RESERVED".into(),
        ]
        .into_iter()
        .collect(),
    );

    #[cfg(feature = "spdk")]
    let src = env::current_dir().unwrap().join("spdk");

    // The bindgen::Builder is the main entry point
    // to bindgen, and lets you build up options for
    // the resulting bindings.
    #[cfg(feature = "spdk")]
    let bindings = bindgen::Builder::default()
        .clang_arg(format!("-I{}", src.join("build/include").display()))
        // The input header we would like to generate bindings for.
        .header("wrapper.h")
        .parse_callbacks(Box::new(ignored_macros))
        // Tell cargo to invalidate the built crate whenever any of the
        // included header files changed.
        // .parse_callbacks(Box::new(bindgen::CargoCallbacks))
        .blocklist_item("IPPORT_.*")
        // XXX: workaround for 'error[E0588]: packed type cannot transitively contain a `#[repr(align)]` type'
        .blocklist_type("spdk_nvme_tcp_rsp")
        .blocklist_type("spdk_nvme_tcp_cmd")
        .blocklist_type("spdk_nvmf_fabric_prop_get_rsp")
        .blocklist_type("spdk_nvmf_fabric_connect_rsp")
        .blocklist_type("spdk_nvmf_fabric_connect_cmd")
        .blocklist_type("spdk_nvmf_fabric_auth_send_cmd")
        .blocklist_type("spdk_nvmf_fabric_auth_recv_cmd")
        .blocklist_type("spdk_nvme_health_information_page")
        .blocklist_type("spdk_nvme_ctrlr_data")
        .blocklist_function("spdk_nvme_ctrlr_get_data")
        .opaque_type("spdk_nvme_sgl_descriptor")
        // Finish the builder and generate the bindings.
        .generate()
        // Unwrap the Result and panic on failure.
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    #[cfg(feature = "spdk")]
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    #[cfg(feature = "spdk")]
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");

    if !Path::new("rocksdb/AUTHORS").exists() {
        update_submodules();
    }
    bindgen_rocksdb();

    if !try_to_find_and_link_lib("ROCKSDB") {
        println!("cargo:rerun-if-changed=rocksdb/");
        fail_on_empty_directory("rocksdb");
        build_rocksdb();
    } else {
        let target = env::var("TARGET").unwrap();
        // according to https://github.com/alexcrichton/cc-rs/blob/master/src/lib.rs#L2189
        if target.contains("apple") || target.contains("freebsd") || target.contains("openbsd") {
            println!("cargo:rustc-link-lib=dylib=c++");
        } else if target.contains("linux") {
            println!("cargo:rustc-link-lib=dylib=stdc++");
        }
    }
    if cfg!(feature = "snappy") && !try_to_find_and_link_lib("SNAPPY") {
        println!("cargo:rerun-if-changed=snappy/");
        fail_on_empty_directory("snappy");
        build_snappy();
    }

    // Allow dependent crates to locate the sources and output directory of
    // this crate. Notably, this allows a dependent crate to locate the RocksDB
    // sources and built archive artifacts provided by this crate.
    println!(
        "cargo:cargo_manifest_dir={}",
        env::var("CARGO_MANIFEST_DIR").unwrap()
    );
    println!("cargo:out_dir={}", env::var("OUT_DIR").unwrap());
}

#[cfg(feature = "spdk")]
#[derive(Debug)]
struct IgnoreMacros(HashSet<String>);

#[cfg(feature = "spdk")]
impl bindgen::callbacks::ParseCallbacks for IgnoreMacros {
    fn will_parse_macro(&self, name: &str) -> bindgen::callbacks::MacroParsingBehavior {
        if self.0.contains(name) {
            bindgen::callbacks::MacroParsingBehavior::Ignore
        } else {
            bindgen::callbacks::MacroParsingBehavior::Default
        }
    }
}
