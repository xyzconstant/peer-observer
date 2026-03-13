fn main() -> Result<(), Box<dyn std::error::Error>> {
    capnpc::CompilerCommand::new()
        .file("capnp/mp/proxy.capnp")
        .file("capnp/chain.capnp")
        .file("capnp/common.capnp")
        .file("capnp/handler.capnp")
        .file("capnp/echo.capnp")
        .file("capnp/mining.capnp")
        .file("capnp/init.capnp")
        .run()?;
    Ok(())
}
