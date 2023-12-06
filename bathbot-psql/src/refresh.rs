use std::time::Duration;

use eyre::Report;
use sqlx::PgPool;

pub(crate) async fn refresh_materialized_views(pool: PgPool) {
    const DAY: Duration = Duration::from_secs(24 * 3600);

    let mut interval = tokio::time::interval(DAY);

    loop {
        interval.tick().await;

        info!("Refreshing materialized views...");

        let mut conn = match pool.acquire().await.map_err(Report::new) {
            Ok(conn) => conn,
            Err(err) => {
                warn!(
                    ?err,
                    "Failed to acquire connection to refresh materialized views"
                );

                continue;
            }
        };

        let user_scores_query =
            sqlx::query!(r#"REFRESH MATERIALIZED VIEW CONCURRENTLY user_scores"#);

        if let Err(err) = user_scores_query.execute(&mut *conn).await {
            warn!(err = ?Report::new(err), "Failed to refresh user_scores materialized view");
        }

        info!("Finished refreshing materialized views");
    }
}
