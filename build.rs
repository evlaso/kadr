fn main() {
    glib_build_tools::compile_resources(&["data"], "data/kadr.gresource.xml", "kadr.gresource");
}
