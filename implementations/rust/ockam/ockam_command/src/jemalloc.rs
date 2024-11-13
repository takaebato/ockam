use std::os::unix::process::CommandExt;

pub fn setup() {
    // we cannot configure jemalloc after it has been initialized,
    // so we need to set the environment variable and re-run the binary
    if std::env::var("_RJEM_MALLOC_CONF").is_err() {
        std::env::set_var(
            "_RJEM_MALLOC_CONF",
            "abort_conf:false,hpa:true,thp:always,metadata_thp:always,percpu_arena:phycpu,background_thread:true",
        );

        // re-exec this binary with the environment variable set
        let mut cmd = std::process::Command::new(std::env::args().next().unwrap());
        cmd.args(std::env::args().skip(1));
        cmd.exec();
    }
}
