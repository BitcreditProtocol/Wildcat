use bcr_wdc_quote_service::ApiDoc;

fn main() {
    let yml = ApiDoc::generate_yml();
    std::fs::write("openapi.yml", yml).unwrap();

    let json = ApiDoc::generate_json();
    std::fs::write("openapi.json", json).unwrap();
}
