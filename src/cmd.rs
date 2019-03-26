use clap::{App, Arg, crate_version, crate_description};

pub fn app() -> App<'static, 'static> {
    let app = App::new("Cortex")
        .version(crate_version!())
        .about(crate_description!())
        .author("Hendrikx ITC <info@hendrikx-itc.nl>")
        .arg(
            Arg::with_name("config")
                .short("c")
                .value_name("CONFIG_FILE")
                .help("Specify config file")
                .takes_value(true),
        );

    app
}
