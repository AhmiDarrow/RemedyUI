//! The isolated-decoding tests (`tests/isolated.rs`) as an Android library. An instrumentation
//! (`tools/IsolatedCheck.java`) loads it and calls `run` with the app's Context; the tests'
//! report comes back as text. `tools/android-check.sh` builds, installs and runs it. Built for
//! any other platform it is empty.

// JNI entry points and descriptor redirection.
#![allow(unsafe_code)]

#[cfg(target_os = "android")]
#[path = "../tests/isolated.rs"]
mod suite;

#[cfg(target_os = "android")]
mod android {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::process::ExitCode;

    use jni::JNIEnv;
    use jni::objects::{JClass, JObject, JString};
    use jni::sys::jstring;

    /// The helper's formats: the hostile ones the tests decode.
    #[unsafe(no_mangle)]
    pub extern "C" fn remedy_ui_media_isolated_formats() {
        super::suite::register_formats();
    }

    fn run(
        env: &mut JNIEnv<'_>,
        context: &JObject<'_>,
        cache: &JString<'_>,
        filter: &JString<'_>,
    ) -> Result<String, String> {
        let cache: String = env.get_string(cache).map_err(|e| e.to_string())?.into();
        let filter: String = env.get_string(filter).map_err(|e| e.to_string())?.into();
        // SAFETY: set before any thread of the tests reads the environment.
        unsafe {
            std::env::set_var("TMPDIR", &cache);
            std::env::set_var("REMEDY_TEST_FILTER", &filter);
        }
        let vm = env.get_java_vm().map_err(|e| e.to_string())?;
        let context = env.new_global_ref(context).map_err(|e| e.to_string())?;
        // SAFETY: the VM pointer is the process's; the context is a global reference kept for
        // the process's life (forgotten below), as android-activity does for an activity.
        unsafe {
            ndk_context::initialize_android_context(
                vm.get_java_vm_pointer().cast(),
                context.as_obj().as_raw().cast(),
            )
        };
        std::mem::forget(context);

        // The tests print; an app's standard output goes nowhere, so it goes to a file.
        let path = format!("{cache}/isolated-check.txt");
        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        // SAFETY: points this process's standard output and error at the report file.
        unsafe {
            libc::dup2(file.as_raw_fd(), 1);
            libc::dup2(file.as_raw_fd(), 2);
        }
        let code = super::suite::main();
        let _ = std::io::stdout().flush();
        let verdict = if code == ExitCode::SUCCESS { "PASSED" } else { "FAILED" };
        let report = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        Ok(format!("{report}\nisolated-check: {verdict}\n"))
    }

    /// `IsolatedCheck.run(context, cacheDir, filter)`: runs the tests, returns their report.
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_remedyui_isolatedcheck_IsolatedCheck_run<'l>(
        mut env: JNIEnv<'l>,
        _class: JClass<'l>,
        context: JObject<'l>,
        cache: JString<'l>,
        filter: JString<'l>,
    ) -> jstring {
        let report = run(&mut env, &context, &cache, &filter)
            .unwrap_or_else(|e| format!("isolated-check: could not run: {e}"));
        env.new_string(report).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
    }
}
