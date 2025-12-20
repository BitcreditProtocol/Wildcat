use bcr_wdc_admin_aggregator::ApiDoc;

fn main() {
    let yml = ApiDoc::generate_yml().unwrap();
    std::fs::write("openapi.yml", yml).unwrap();

    let json = ApiDoc::generate_json().unwrap();
    std::fs::write("openapi.json", json).unwrap();
}
