// mod util;

use rocksdb::*;

fn main() {
    // let env = Env::rocksdb_create_spdk_env(
    //     &std::env::args().nth(1).expect("Please specify directory"),
    //     &std::env::args().nth(2).expect("No config file"),
    //     &std::env::args().nth(3).expect("Need bdev name"),
    //     4096,
    // ).expect("fail to initialize spdk env");
    println!("test foo bar");
    let env = Env::rocksdb_create_spdk_env(
            "spdk_integration_test_dir",
            "rocksdb_spdk.json",
            "Nvme1n1",
            4096,
        ).expect("fail to initialize spdk env");
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.set_env(&env);
    let path = "spdk_integration_test_dir";
    let db = DB::open(&opts, path).expect("fail to open db");
    db.put(b"foo", b"bar").expect("fail to put");
    println!("put succeed!");
    match db.get(b"foo"){
        Ok(Some(res)) =>{
            println!("got value {:?} succeed!", String::from_utf8(res).unwrap());
        },
        Ok(None) =>{
            println!("got none value");
        },
        Err(e) =>{
            println!("err: {:?}", e);
        },
    };
    println!("Test SPDK Integration Succeed!");
    // drop(db);
    // drop(env);
}
