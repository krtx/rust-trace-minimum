use tracing::Instrument;
use tracing_core::Level;
use tracing_subscriber::{util::SubscriberInitExt, layer::{Layer, SubscriberExt}};
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{Resource, trace::{Sampler, RandomIdGenerator}};
use opentelemetry_semantic_conventions::{
    attribute::{SERVICE_NAME, SERVICE_VERSION},
    SCHEMA_URL,
};

#[derive(Clone)]
struct AppState {
    pool: sqlx::MySqlPool,
}

#[tokio::main]
async fn main() {
    // Tracer setup
    let resource = Resource::from_schema_url(
        [
            opentelemetry::KeyValue::new(SERVICE_NAME, env!("CARGO_PKG_NAME")),
            opentelemetry::KeyValue::new(SERVICE_VERSION, env!("CARGO_PKG_VERSION")),
        ],
        SCHEMA_URL,
    );

    let provider = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(opentelemetry_otlp::new_exporter().tonic().with_endpoint("http://localhost:4317"))
        .with_trace_config(
            opentelemetry_sdk::trace::Config::default()
                // sampling rate
                .with_sampler(Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(1.0))))

                // id generator
                .with_id_generator(RandomIdGenerator::default())

                // resource
                .with_resource(resource)
        )
        .install_batch(opentelemetry_sdk::runtime::Tokio)
        .unwrap();

    let tracer = provider.tracer(env!("CARGO_PKG_NAME"));

    tracing_subscriber::registry()
        // global filter to hide h2 traces
        .with(tracing_subscriber::filter::LevelFilter::from_level(Level::INFO))

        // stdout log (severity >= WARN)
        .with(tracing_subscriber::fmt::layer().with_filter(tracing_subscriber::filter::LevelFilter::WARN))

        // opentelemetry log (severity >= INFO)
        .with(tracing_opentelemetry::OpenTelemetryLayer::new(tracer))
        .init();

    // DB setup
    let options = sqlx::mysql::MySqlConnectOptions::new()
        .host("127.0.0.1")
        .port(3306)
        .username("user")
        .password("password")
        .database("mydb");

    let pool = sqlx::mysql::MySqlPoolOptions::new()
        .connect_with(options)
        .await
        .expect("Failed to connect to MySQL");
    
    // Server setup
    let app = axum::Router::new()
        .route("/", axum::routing::get(root))
        .route("/cause_error", axum::routing::get(cause_error))
        .with_state(AppState{ pool });

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app)
        .await
        .unwrap();
}

#[tracing::instrument]
async fn root(axum::extract::State(AppState { pool }): axum::extract::State<AppState>) -> &'static str {

    // Emit an info level event
    tracing::info!("Processing request");

    // Synchronous function call can be wrapped with `info_span` macro
    let _ = tracing::info_span!("some process").in_scope(|| {
        bcrypt::hash("password", 4)
    });

    // Asynchronous function call can be added with `instrument` method
    let _ = sqlx::query("SELECT 1 + 1 as result")
        .fetch_one(&pool)
        .instrument(tracing::info_span!("fetch row"))
        .await
        .expect("Failed to fetch row");
    
    "ok"
}

#[tracing::instrument]
async fn cause_error(axum::extract::State(AppState { pool }): axum::extract::State<AppState>) -> &'static str {

    // This event won't be shown in the stdout log, but will be shown in the opentelemetry log
    tracing::info!("Processing request");

    // This event will be shown in the stdout log, and will be shown in the opentelemetry log
    tracing::warn!("possible error");

    let rs = sqlx::query("SQL SYNTAX ERROR")
        .fetch_one(&pool)
        .instrument(tracing::info_span!("fetch row"))
        .await;

    match rs {
        Ok(_) => (),
        Err(e) => {
            // This event will be shown in the stdout log, and will be shown in the opentelemetry log
            tracing::error!("Error: {:?}", e);
        }
    }
    
    "ok"
}