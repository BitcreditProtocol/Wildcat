// ----- standard library imports
use std::time::Duration;
// ----- extra library imports
use bcr_ebill_api::util::date::now;
use bcr_wdc_ebill_service::AppController;
use log::{error, info};
use tokio::time::{interval, sleep};
// ----- end imports

pub async fn run(
    app: AppController,
    job_runner_initial_delay_seconds: u64,
    job_runner_check_interval_seconds: u64,
) {
    sleep(Duration::from_secs(job_runner_initial_delay_seconds)).await;
    info!(
        "Job runner started after {job_runner_initial_delay_seconds}s of initial delay, running jobs every {job_runner_check_interval_seconds}s...",
    );

    let mut check_interval_tick = interval(Duration::from_secs(job_runner_check_interval_seconds));
    loop {
        check_interval_tick.tick().await;
        run_jobs(&app.clone()).await;
    }
}

async fn run_jobs(app: &AppController) {
    tokio::join!(
        run_check_bill_payment_job(app.clone()),
        run_check_bill_offer_to_sell_payment_job(app.clone()),
        run_check_bill_recourse_payment_job(app.clone())
    );
    // explicitly not added to join! because we want to run this job after
    // all payment jobs are done and avoid any concurrency issues.
    run_check_bill_timeouts(app.clone()).await;
}

async fn run_check_bill_payment_job(app: AppController) {
    info!("Running Check Bill Payment Job");
    if let Err(e) = app.bill_service.check_bills_payment().await {
        error!("Error while running Check Bill Payment Job: {e}");
    }
    info!("Finished running Check Bill Payment Job");
}

async fn run_check_bill_offer_to_sell_payment_job(app: AppController) {
    info!("Running Check Bill Offer to Sell Payment Job");
    if let Err(e) = app.bill_service.check_bills_offer_to_sell_payment().await {
        error!("Error while running Check Bill Offer to Sell Payment Job: {e}");
    }
    info!("Finished running Check Bill Offer to Sell Payment Job");
}

async fn run_check_bill_recourse_payment_job(app: AppController) {
    info!("Running Check Bill Recourse Payment Job");
    if let Err(e) = app.bill_service.check_bills_in_recourse_payment().await {
        error!("Error while running Check Bill Recourse Payment Job: {e}");
    }
    info!("Finished running Check Bill Recourse Payment Job");
}

async fn run_check_bill_timeouts(app: AppController) {
    info!("Running Check Bill Timeouts Job");
    let current_time = now().timestamp();
    if let Err(e) = app
        .bill_service
        .check_bills_timeouts(current_time as u64)
        .await
    {
        error!("Error while running Check Bill Timeouts Job: {e}");
    }

    info!("Finished running Check Bill Timeouts Job");
}
