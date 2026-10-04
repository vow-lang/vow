# [0.10.0](https://github.com/vow-lang/vow/compare/v0.9.0...v0.10.0) (2026-10-04)


### Bug Fixes

* **checker:** reject non-bool if conditions in self-hosted checker ([#1453](https://github.com/vow-lang/vow/issues/1453)) ([db4d719](https://github.com/vow-lang/vow/commit/db4d719a859758953a6cc78bd5c98b0c652986ea))
* **compiler:** anchor self-hosted linear/module/string-literal diagnostics on real spans ([#1365](https://github.com/vow-lang/vow/issues/1365)) ([d4e51e3](https://github.com/vow-lang/vow/commit/d4e51e34041508542459b566e5df5854d475b45b))
* **lower:** anchor self-hosted contract source.offset on the clause keyword ([#1367](https://github.com/vow-lang/vow/issues/1367)) ([d363e9f](https://github.com/vow-lang/vow/commit/d363e9ffbbf77cc072b793c44be53856a387e9ed))
* **lower:** anchor self-hosted where-refinement source.offset on param name ([#1386](https://github.com/vow-lang/vow/issues/1386)) ([1d2b341](https://github.com/vow-lang/vow/commit/1d2b341813e462e950b0bb0c0fee01c79a0adb41))
* **lower:** anchor spans on generic enum-constructor allocations ([#1438](https://github.com/vow-lang/vow/issues/1438)) ([791ec7a](https://github.com/vow-lang/vow/commit/791ec7a93912b749f51f40a4f433946b47c4eb2e))
* **lower:** apply narrow-integer contextual width lowering uniformly ([#1439](https://github.com/vow-lang/vow/issues/1439)) ([63ee6d8](https://github.com/vow-lang/vow/commit/63ee6d884c537ef1dff1c4f938f3da24ac3eccfd))
* **lower:** print contract text identically in both compilers ([#1482](https://github.com/vow-lang/vow/issues/1482)) ([cf9434c](https://github.com/vow-lang/vow/commit/cf9434cfee342b212e20316323d3db9d1e17da59))
* **lower:** re-narrow if-expression marker branches to merge width ([#1394](https://github.com/vow-lang/vow/issues/1394)) ([b64bf73](https://github.com/vow-lang/vow/commit/b64bf731e959d6b9b0b04a24a070df5a11c99c4a))
* **mutants:** stop vowc mutants' Tier-2 oracle from rebuilding the Rust compiler ([#1456](https://github.com/vow-lang/vow/issues/1456)) ([00e18ac](https://github.com/vow-lang/vow/commit/00e18ac0c5d92a11686e8634d981e2372f53622b))
* **pair-review:** report a one-sided soundness gate failure beside a confirmed finding ([#1442](https://github.com/vow-lang/vow/issues/1442)) ([f1758f9](https://github.com/vow-lang/vow/commit/f1758f95b0a4dd411bc6883ec37b2637e9ee65e7))
* **parser:** drop span_pack's verifier-bound start<65536 requires ([#1360](https://github.com/vow-lang/vow/issues/1360)) ([db008a1](https://github.com/vow-lang/vow/commit/db008a1d666c37e66ea1627b666a4da6dd3cfb22))
* **parser:** span every self-hosted expression and type node ([#1355](https://github.com/vow-lang/vow/issues/1355)) ([1fe178f](https://github.com/vow-lang/vow/commit/1fe178f72567dfa034a732af7552a73562c5d9dd))
* **region:** classify BTreeMap and parse_opt externs as heap-producing ([#1444](https://github.com/vow-lang/vow/issues/1444)) ([7f142ad](https://github.com/vow-lang/vow/commit/7f142ad3884cf6a2fb74c983ab2a8bac166dacce))
* **runtime:** route map and fresh-aggregate builtins through inferred arenas ([#1481](https://github.com/vow-lang/vow/issues/1481)) ([0a3a4f8](https://github.com/vow-lang/vow/commit/0a3a4f808027730b55ca62314d15ec1e09c548a6))
* **symphonika:** supply real newlines in impl.md's PR body ([#1441](https://github.com/vow-lang/vow/issues/1441)) ([898a426](https://github.com/vow-lang/vow/commit/898a426aa2b34d534d93c525fb7b89b068203e0a))
* **syntax:** reject f64 literals that overflow to infinity ([#1354](https://github.com/vow-lang/vow/issues/1354)) ([63b2bea](https://github.com/vow-lang/vow/commit/63b2bea87c857c87090b80c3e84071aa5ef0fa40))
* **test:** run test binaries directly and report signal deaths as failures ([#1349](https://github.com/vow-lang/vow/issues/1349)) ([636271c](https://github.com/vow-lang/vow/commit/636271c1e11e3242169dd445fc657993e12d61f0))
* **test:** treat one-sided empty compiler output as FAIL, not SKIP ([#1466](https://github.com/vow-lang/vow/issues/1466)) ([39ca8a4](https://github.com/vow-lang/vow/commit/39ca8a4fcffbf09c4cc168cf550401d55fade137))
* **types:** accept every integer type for const declarations in both compilers ([#1470](https://github.com/vow-lang/vow/issues/1470)) ([e9215be](https://github.com/vow-lang/vow/commit/e9215bed47a96c72105ca110a24beb76cd987844))
* **types:** accept u32 shift counts for 64/128-bit values in both compilers ([#1471](https://github.com/vow-lang/vow/issues/1471)) ([8081b01](https://github.com/vow-lang/vow/commit/8081b016597b303d32527f1a7788d65b0b46bccd))
* **types:** make HashMap::get return Option<V> in both compilers ([#1480](https://github.com/vow-lang/vow/issues/1480)) ([e502ea5](https://github.com/vow-lang/vow/commit/e502ea58a581a5e4148847f1d5ed8ec1eeb42697))
* **types:** reject checked arithmetic on float operands ([#1377](https://github.com/vow-lang/vow/issues/1377)) ([6addc51](https://github.com/vow-lang/vow/commit/6addc51ebfb30a6568fd6d636e71a16ea9406030))
* **types:** reject contract clauses that write through a shared argument ([#1455](https://github.com/vow-lang/vow/issues/1455)) ([cf03350](https://github.com/vow-lang/vow/commit/cf03350a675feb06325147a7cd8175aa5b92457c))
* **types:** reject unsupported map key/value types and map indexing ([#1483](https://github.com/vow-lang/vow/issues/1483)) ([7f35380](https://github.com/vow-lang/vow/commit/7f3538000e95889969f69878112a699357459484))
* **types:** reject Vec::get in both compilers instead of miscompiling it ([#1469](https://github.com/vow-lang/vow/issues/1469)) ([9b81f9d](https://github.com/vow-lang/vow/commit/9b81f9d6a635d9609d3ecb49b5b0a8de33a8ea8c))
* **verify:** cover i64/u64/i128/u128 dynamic shift-count asserts ([#1395](https://github.com/vow-lang/vow/issues/1395)) ([82e4a71](https://github.com/vow-lang/vow/commit/82e4a71fe63e8ce02f230c2e3b3cf79725b33404))
* **verify:** fail closed on non-finite ConstF64 in both C emitters ([#1379](https://github.com/vow-lang/vow/issues/1379)) ([aaa257a](https://github.com/vow-lang/vow/commit/aaa257abc62d8a1d17e660b789c3f49a713c24d5))
* **verify:** reject a tainted VERIFICATION SUCCESSFUL from ESBMC ([#1362](https://github.com/vow-lang/vow/issues/1362)) ([8b12f0a](https://github.com/vow-lang/vow/commit/8b12f0adb58df40b5c47649e7d24f4a871120cf5))
* **verify:** report bounded proofs, fold constant lengths, model null source ([#1484](https://github.com/vow-lang/vow/issues/1484)) ([24c16ee](https://github.com/vow-lang/vow/commit/24c16ee80a3cc3272c8a788c60a18f3ca2f44184))


### Features

* **catalogue:** harden Operation Catalogue validation and diagnostics ([#1358](https://github.com/vow-lang/vow/issues/1358)) ([b2666c3](https://github.com/vow-lang/vow/commit/b2666c3b82527de6d85b8e3229cd2667ef73ffb3))
* **catalogue:** migrate fs/stdin/args/stderr builtins to operation catalogue ([#1356](https://github.com/vow-lang/vow/issues/1356)) ([8a3e466](https://github.com/vow-lang/vow/commit/8a3e4669338818d1fa748edc225d5e7a342d3675))
* **mutants:** add a fast Tier-1.5 checkpoint before the full Tier-2 suite ([#1467](https://github.com/vow-lang/vow/issues/1467)) ([5a1fa3d](https://github.com/vow-lang/vow/commit/5a1fa3d8d737f004235f64fc6045a63c6579fc7e))
* **types:** flip len() to u64 in both compilers and the C model ([#1443](https://github.com/vow-lang/vow/issues/1443)) ([cfadb52](https://github.com/vow-lang/vow/commit/cfadb523765f929654383712783b4ce2f6b5ea4f))
* **types:** make from_raw_parts_copy length arguments u64 ([#1448](https://github.com/vow-lang/vow/issues/1448)) ([1857cf2](https://github.com/vow-lang/vow/commit/1857cf235308fe115f7fb89ca3e13babb35eb5ad))
* **types:** require u64 for vec indices in both compilers ([#1468](https://github.com/vow-lang/vow/issues/1468)) ([623ec62](https://github.com/vow-lang/vow/commit/623ec62ff45c0abbe77beba128c265a7b380f2da))
* **types:** require u64 String offsets and a u8 push_byte in both compilers ([#1486](https://github.com/vow-lang/vow/issues/1486)) ([f010efe](https://github.com/vow-lang/vow/commit/f010efeaae586c8aa8a10ef504222176ed511922))
* **vow-perf:** add recommended_grid to mitigate threshold-plateau false fails ([#1396](https://github.com/vow-lang/vow/issues/1396)) ([c0134d3](https://github.com/vow-lang/vow/commit/c0134d36248666579506124b3d5237b9ecd7adeb))

# [0.9.0](https://github.com/vow-lang/vow/compare/v0.8.0...v0.9.0) (2026-09-27)


### Bug Fixes

* **checker:** reject type-mismatched assignments in self-hosted checker ([#1264](https://github.com/vow-lang/vow/issues/1264)) ([ed4c9b9](https://github.com/vow-lang/vow/commit/ed4c9b9eb46dce02ad6224debd45e4a0e51ea4da))
* **checker:** validate struct-literal field types and names ([#1278](https://github.com/vow-lang/vow/issues/1278)) ([fab6d1d](https://github.com/vow-lang/vow/commit/fab6d1d13f369ff22c55c5329cfd6e9ae916eea4))
* **ci:** check exact help-JSON fields instead of a flattened blob ([#1258](https://github.com/vow-lang/vow/issues/1258)) ([9eecc3d](https://github.com/vow-lang/vow/commit/9eecc3d14784b6dbdf6583df2db7b96e63d70ed1))
* **diag:** anchor call/enum-ctor/builtin-method spans correctly ([#1263](https://github.com/vow-lang/vow/issues/1263)) ([0762f00](https://github.com/vow-lang/vow/commit/0762f0022557e60728de542fa88c5afabc930b01))
* **docs:** mask code spans and fenced blocks before escaping-link rewrite ([#1250](https://github.com/vow-lang/vow/issues/1250)) ([d79ae6e](https://github.com/vow-lang/vow/commit/d79ae6ea4584c45f6a3b69ec08fcfdee261e1b3f))
* **lexer:** use checked subtraction in span_len instead of dropping its contract ([#1306](https://github.com/vow-lang/vow/issues/1306)) ([3f41dfc](https://github.com/vow-lang/vow/commit/3f41dfc837d1412585e90d93caba1ac94ee2b194))
* **lower:** tag proc_sample as heap-returning in tag_builtin_result ([#1288](https://github.com/vow-lang/vow/issues/1288)) ([f3182d1](https://github.com/vow-lang/vow/commit/f3182d19d8bf9fdc9bcb1ccec462236f06a23261))
* port symphonika's blocked-sentinel gate, trim impl.md ([#1284](https://github.com/vow-lang/vow/issues/1284)) ([e52e690](https://github.com/vow-lang/vow/commit/e52e69043ba8a34387f7a0843194492554525541))
* **symphonika:** gate implement stage on an open PR, not just a commit ([#1280](https://github.com/vow-lang/vow/issues/1280)) ([9befaad](https://github.com/vow-lang/vow/commit/9befaad97027afe41f8ce166cecb4b1694744282))
* **verify:** port parse_u64_opt verifier model to self-hosted emitter ([#1289](https://github.com/vow-lang/vow/issues/1289)) ([4b37ef2](https://github.com/vow-lang/vow/commit/4b37ef2a26a0c42a9b14d18b27d7d63b127a3b8a))


### Features

* **compiler:** implement let (a, b) tuple destructuring ([#1333](https://github.com/vow-lang/vow/issues/1333)) ([0e4059f](https://github.com/vow-lang/vow/commit/0e4059f7609d0f6281e60cffd04a66f0de63b1c6))

# [0.8.0](https://github.com/vow-lang/vow/compare/v0.7.0...v0.8.0) (2026-09-06)


### Bug Fixes

* **clif-shim:** give each Phi a shadow slot so back-edge Upsilons copy in parallel ([#1187](https://github.com/vow-lang/vow/issues/1187)) ([df01d57](https://github.com/vow-lang/vow/commit/df01d57c876fac6d0eefb396b596477c843da275))
* **codegen:** align object writes on missing output directories ([#1221](https://github.com/vow-lang/vow/issues/1221)) ([cddc635](https://github.com/vow-lang/vow/commit/cddc6355c820fc8c0546b6cd9593dea3bc438977))
* **codegen:** diagnose unchecked division aborts ([#1206](https://github.com/vow-lang/vow/issues/1206)) ([b0fa7c7](https://github.com/vow-lang/vow/commit/b0fa7c781a1915835c7bda5841c68b87f7e603a4))
* **codegen:** emit structured diagnostics for backend failures ([#1163](https://github.com/vow-lang/vow/issues/1163)) ([9b65f58](https://github.com/vow-lang/vow/commit/9b65f5809ee7452dc07d8665f3f123df1e9fdfaf))
* **codegen:** preserve full i128 and u128 VowViolation binding values ([#1166](https://github.com/vow-lang/vow/issues/1166)) ([03bae56](https://github.com/vow-lang/vow/commit/03bae561fd3abaa40c3b05512830ea24386c6dd7))
* **codegen:** reject 128-bit struct fields and enum payloads instead of truncating ([#1168](https://github.com/vow-lang/vow/issues/1168)) ([8e0c98d](https://github.com/vow-lang/vow/commit/8e0c98d9d29920eb5aeade90f22441f1848bc6e4))
* **codegen:** report checked-arithmetic overflow in every build mode ([#1137](https://github.com/vow-lang/vow/issues/1137)) ([35a6a27](https://github.com/vow-lang/vow/commit/35a6a273a84757fa5e335cc7a9ea6dfadbb7c2b8))
* **compiler:** create output parent directories in self-hosted builds ([#1219](https://github.com/vow-lang/vow/issues/1219)) ([703ab9b](https://github.com/vow-lang/vow/commit/703ab9bc30fe512a03daac6f522b7fdb4f3bd044))
* **compiler:** lex bare float literals in the self-hosted compiler ([#1254](https://github.com/vow-lang/vow/issues/1254)) ([435d2aa](https://github.com/vow-lang/vow/commit/435d2aa5ec9381a0ccb5c3a281a7d5ede3f2afbc))
* **compiler:** resolve cast target types in the self-hosted contract printer ([#1157](https://github.com/vow-lang/vow/issues/1157)) ([19c9538](https://github.com/vow-lang/vow/commit/19c9538e804619f5ecbc47449fba1da627cffd69))
* **contracts:** close the as-cast bypass in tautological and weak clause detection ([#1142](https://github.com/vow-lang/vow/issues/1142)) ([a25de4e](https://github.com/vow-lang/vow/commit/a25de4ebb62a2aaa7692453354b22837c0590daf))
* **docs:** correct broken examples.md anchor and let --strict catch anchors ([#1178](https://github.com/vow-lang/vow/issues/1178)) ([bd0fdf9](https://github.com/vow-lang/vow/commit/bd0fdf9d543f9eed165313a988ce121dcef8ae24))
* **docs:** retarget every escaping spec link so the site builds under --strict ([#1176](https://github.com/vow-lang/vow/issues/1176)) ([8677236](https://github.com/vow-lang/vow/commit/8677236e916e2938e820d8cc9a46e38543d8dae1))
* **docs:** rewrite reference-style ../ links in copied spec pages ([#1249](https://github.com/vow-lang/vow/issues/1249)) ([efb764e](https://github.com/vow-lang/vow/commit/efb764e427ac48bb8e4f46b5366e167ebc2a7ccc))
* **docs:** validate fragment anchors on externalized ../ spec links ([#1251](https://github.com/vow-lang/vow/issues/1251)) ([238f18a](https://github.com/vow-lang/vow/commit/238f18aa0b15740400cc130ae12bc45969740be4))
* **frontend:** stop type-checking after module-load errors ([#1223](https://github.com/vow-lang/vow/issues/1223)) ([0f5c1c5](https://github.com/vow-lang/vow/commit/0f5c1c5dbaed08a09b2e2480de33fd44a7f3b53c))
* **ir:** lower float arithmetic with float opcodes ([#1217](https://github.com/vow-lang/vow/issues/1217)) ([48e1180](https://github.com/vow-lang/vow/commit/48e1180cdd9f89e5b3a0bacb66444cb429f42bd2))
* **ir:** preserve narrow Vec width in index expressions ([#1195](https://github.com/vow-lang/vow/issues/1195)) ([294c674](https://github.com/vow-lang/vow/commit/294c67498559379ef8a51669c3cdc4f67c2c41e0))
* **ir:** preserve u64 checked literal overflow semantics ([#1205](https://github.com/vow-lang/vow/issues/1205)) ([df60cb8](https://github.com/vow-lang/vow/commit/df60cb87cb1a30dc222899e4f605854ef43b44e4))
* **lexer:** drop dead .. token and reject usize/isize literal suffixes ([#1145](https://github.com/vow-lang/vow/issues/1145)) ([c0f927c](https://github.com/vow-lang/vow/commit/c0f927cdf8b0e72b59c6034ff3b8689ab6cf6265))
* **parity:** compare error codes, blame, and counterexample values ([#1149](https://github.com/vow-lang/vow/issues/1149)) ([8c10e8d](https://github.com/vow-lang/vow/commit/8c10e8d2622107ff9d4340eb85fd0874e2a9a4ab))
* **parser:** support extern "C" blocks in self-hosted compiler ([#1189](https://github.com/vow-lang/vow/issues/1189)) ([c4d4471](https://github.com/vow-lang/vow/commit/c4d4471c3c06590df71810efae7a647bcd54982d))
* **replay:** distinguish runtime aborts from counterexample divergence ([#1192](https://github.com/vow-lang/vow/issues/1192)) ([ed71363](https://github.com/vow-lang/vow/commit/ed713636c79213f7bd7db1ffef1f8841406e4739))
* **scripts:** clean up scratch trees on SIGTERM, SIGINT and SIGHUP ([#1133](https://github.com/vow-lang/vow/issues/1133)) ([0f642a2](https://github.com/vow-lang/vow/commit/0f642a296a9ea472b86fbcb95168964b378b2be3))
* **types:** enforce index and builtin-method argument types ([#1125](https://github.com/vow-lang/vow/issues/1125)) ([d14971f](https://github.com/vow-lang/vow/commit/d14971f447b29b5caeff4f5cb409939bf668b40d))
* **verify:** accept ESBMC 8.4+ multi-property output, bump to 8.5, speed up CI 3.5x ([#1134](https://github.com/vow-lang/vow/issues/1134)) ([c8261fa](https://github.com/vow-lang/vow/commit/c8261fa81449cf030e4c00a016f4c61132ff4026))
* **verify:** align multi-function counterexample reporting ([#1215](https://github.com/vow-lang/vow/issues/1215)) ([6e232df](https://github.com/vow-lang/vow/commit/6e232df1a70d83cfcbb64d6f6c560db2cb6afe4f))
* **verify:** describe non-contract counterexamples ([#1204](https://github.com/vow-lang/vow/issues/1204)) ([963262c](https://github.com/vow-lang/vow/commit/963262cf44a061ed2d73016974b83a25978fe7dd))
* **verify:** key container bounds asserts off the index IR type ([#1146](https://github.com/vow-lang/vow/issues/1146)) ([4168f64](https://github.com/vow-lang/vow/commit/4168f6434fa7403d300a0f582827ae0beb9e77ab))
* **verify:** make arena solver flags overridable ([#1228](https://github.com/vow-lang/vow/issues/1228)) ([4692fb4](https://github.com/vow-lang/vow/commit/4692fb4b42ef0c75cdec8eb22af49bb71ce007a2))
* **verify:** model checked-arithmetic abort so `+!` is not identical to `+` ([#1150](https://github.com/vow-lang/vow/issues/1150)) ([078674e](https://github.com/vow-lang/vow/commit/078674e545a0a26382e9b641abe23de70e976fb3))
* **verify:** report unattributed ESBMC properties without contract blame ([#1207](https://github.com/vow-lang/vow/issues/1207)) ([3357b9a](https://github.com/vow-lang/vow/commit/3357b9a56839e97147e0e415398f6f37fb0b2af5))
* **verify:** unblock ESBMC verification on macOS ([#1245](https://github.com/vow-lang/vow/issues/1245)) ([7be7c25](https://github.com/vow-lang/vow/commit/7be7c2561d03bfecf33f1b5fdb640f0d0cfa2991))
* **vow-runtime:** guard arena and chunk-header derefs against null pointers ([#1128](https://github.com/vow-lang/vow/issues/1128)) ([7132e4b](https://github.com/vow-lang/vow/commit/7132e4b397e9f4aee154ddd6576adef09ed46d83))


### Features

* **equivalence:** preserve expected classification when extending a ledger entry ([#1246](https://github.com/vow-lang/vow/issues/1246)) ([68a8b65](https://github.com/vow-lang/vow/commit/68a8b65ac096c424d3a06da94cee059bcd426a5d))
* **pair-review:** review compiler pairs in complete chunks ([#1172](https://github.com/vow-lang/vow/issues/1172)) ([0b80fe0](https://github.com/vow-lang/vow/commit/0b80fe0838cab91f31a46e8312fac795a7c67f5d))
* **scripts:** adversarial AI pair review with a mechanical confirmation gate ([#1091](https://github.com/vow-lang/vow/issues/1091)) ([c8733b1](https://github.com/vow-lang/vow/commit/c8733b107b9cd78a68491bce1a250992ba61aa6f))
* **scripts:** differential equivalence runner + nightly sweep ([#1086](https://github.com/vow-lang/vow/issues/1086)) ([7172e9e](https://github.com/vow-lang/vow/commit/7172e9e4e3b40b2ce95827519a437235c62cf94b))
* **scripts:** verifier-model vs concrete-runtime differential (finds a false Verified) ([#1092](https://github.com/vow-lang/vow/issues/1092)) ([a803e03](https://github.com/vow-lang/vow/commit/a803e03c306375682b1beabfb5642a62c5d2e6dc))
* **types:** reject tautological unsigned comparisons with 0 ([#1129](https://github.com/vow-lang/vow/issues/1129)) ([be8b916](https://github.com/vow-lang/vow/commit/be8b916b9ccf9292a5792f16acdf9a7647ab22b8))


### Performance Improvements

* **c-emitter:** cache instruction types per function ([#1210](https://github.com/vow-lang/vow/issues/1210)) ([01cc2a8](https://github.com/vow-lang/vow/commit/01cc2a8fe17015e7e525ff3a3d3c1959abd62710))

# [0.7.0](https://github.com/vow-lang/vow/compare/v0.6.0...v0.7.0) (2026-08-30)


### Bug Fixes

* **compiler:** prefer declaration stubs in self-hosted loader ([#1072](https://github.com/vow-lang/vow/issues/1072)) ([3636203](https://github.com/vow-lang/vow/commit/3636203c63edb8965fde3226b346929dbbeba351))
* **ir:** lower .unwrap() to a tag check instead of ConstUnit ([#1123](https://github.com/vow-lang/vow/issues/1123)) ([8cb5169](https://github.com/vow-lang/vow/commit/8cb5169d44db5298c7d2d19d690e2ad589a9894c))
* **lower:** preserve aggregate types in match payload bindings ([#1020](https://github.com/vow-lang/vow/issues/1020)) ([f280f31](https://github.com/vow-lang/vow/commit/f280f31247d4dc959501b639c0670fcdb45ec951))
* **types:** admit full-range i128 and u128 literals ([#1071](https://github.com/vow-lang/vow/issues/1071)) ([c265799](https://github.com/vow-lang/vow/commit/c2657994a230edffb3399f0469bdfd017bc3fb8c))
* **vow-perf:** count Vec sort runtime work ([#1070](https://github.com/vow-lang/vow/issues/1070)) ([fd91dc8](https://github.com/vow-lang/vow/commit/fd91dc873827d52f154ef954d135d2d327a33617))
* **vow-runtime:** escape VowViolation JSON strings ([#1069](https://github.com/vow-lang/vow/issues/1069)) ([23dc672](https://github.com/vow-lang/vow/commit/23dc6729fc969c49ea79a87de4e2d7f526730837))


### Features

* **codegen:** Cranelift I128 codegen for basic ops ([#1079](https://github.com/vow-lang/vow/issues/1079)) ([05cd632](https://github.com/vow-lang/vow/commit/05cd63276b86f0525a096d6ce78d0db76ba91f7a))
* **codegen:** recover safe projection arena routing ([#999](https://github.com/vow-lang/vow/issues/999)) ([b57db6b](https://github.com/vow-lang/vow/commit/b57db6b006b337ce7aff12fe848c1539b9fbe28d))
* **codegen:** route I128 div/rem/checked-mul through runtime ([#1085](https://github.com/vow-lang/vow/issues/1085)) ([2feeb7d](https://github.com/vow-lang/vow/commit/2feeb7d6fd8af26adb792fa42da8adeacde5803e))
* **ir:** add two-limb i128 and u128 literal constants ([#1063](https://github.com/vow-lang/vow/issues/1063)) ([1dbef4d](https://github.com/vow-lang/vow/commit/1dbef4daa38064a8e4ae99241e1d4070a3993494))
* **numeric:** make remaining narrow integers first-class ([#995](https://github.com/vow-lang/vow/issues/995)) ([f5d2a16](https://github.com/vow-lang/vow/commit/f5d2a16e8e5ecece6d4c774e5f5768d4684d1e49)), closes [#1030](https://github.com/vow-lang/vow/issues/1030) [1030#issuecomment-5312897156](https://github.com/1030/issues/issuecomment-5312897156) [#1030](https://github.com/vow-lang/vow/issues/1030) [3/#4](https://github.com/vow-lang/vow/issues/4) [#5](https://github.com/vow-lang/vow/issues/5) [980/#983](https://github.com/vow-lang/vow/issues/983)

# [0.6.0](https://github.com/vow-lang/vow/compare/v0.5.1...v0.6.0) (2026-08-09)


### Bug Fixes

* **bench:** reject changed skeleton contracts ([#990](https://github.com/vow-lang/vow/issues/990)) ([6d88540](https://github.com/vow-lang/vow/commit/6d88540c27c52d375dd788bcc8f1c46ec4ce2af8))
* **checker:** guard parallel item file lengths ([#1017](https://github.com/vow-lang/vow/issues/1017)) ([eeaa8d0](https://github.com/vow-lang/vow/commit/eeaa8d0ac00295e90d353bedef9dc000af742bb5))
* **ci:** cap build-and-test job runtimes ([#988](https://github.com/vow-lang/vow/issues/988)) ([30036b2](https://github.com/vow-lang/vow/commit/30036b26e01cb518c37e2c5b08cab1f5daa7d80c))
* **euler:** mock LLM SDK imports in resolve_compiler test ([#1035](https://github.com/vow-lang/vow/issues/1035)) ([8c2199d](https://github.com/vow-lang/vow/commit/8c2199d7c387e211623d5b2cbd54bd38826433bd))
* **euler:** use canonical self-hosted compiler path ([#1012](https://github.com/vow-lang/vow/issues/1012)) ([ee96826](https://github.com/vow-lang/vow/commit/ee9682651aec3e5fc5fc282cbed36a605e4b982c))
* **full-test:** report bootstrap stage failures ([#1029](https://github.com/vow-lang/vow/issues/1029)) ([efe5d88](https://github.com/vow-lang/vow/commit/efe5d88ee1a246b20822c7fb0a097999df7e5b77))
* **numeric:** return Option from parse_i64 ([#1023](https://github.com/vow-lang/vow/issues/1023)) ([ab31c9e](https://github.com/vow-lang/vow/commit/ab31c9e0f63cc2254da765a11a9b7870c6acaa97))
* **release:** align confirmation hint with accepted inputs ([#1013](https://github.com/vow-lang/vow/issues/1013)) ([321e571](https://github.com/vow-lang/vow/commit/321e571d5f184f846648265145f0fd2bf2a09ee0))
* **scripts:** omit unmeasured bootstrap RSS field ([#1003](https://github.com/vow-lang/vow/issues/1003)) ([289ebd4](https://github.com/vow-lang/vow/commit/289ebd4fe164d65b10ef3ae8489ab55e5d765ef2))
* **self-hosted:** anchor constructor call spans ([#991](https://github.com/vow-lang/vow/issues/991)) ([1c90bf3](https://github.com/vow-lang/vow/commit/1c90bf38a650798c95b633bd44effd7a6c011505))


### Features

* **format:** implement baseline integer formatters ([#1025](https://github.com/vow-lang/vow/issues/1025)) ([8b868bc](https://github.com/vow-lang/vow/commit/8b868bcdeba14e7eb783629810758148668bcf4c))
* **vow-perf:** classify log-polynomial complexity ([#989](https://github.com/vow-lang/vow/issues/989)) ([07500b9](https://github.com/vow-lang/vow/commit/07500b9a5d87f658526af4c2515f228360054b82))
* **vow-perf:** isolate instrumented compilation artifacts ([#1011](https://github.com/vow-lang/vow/issues/1011)) ([2464f9b](https://github.com/vow-lang/vow/commit/2464f9b281f1a4ba476b9bd67904df7e5420d8bb))


### Performance Improvements

* **bench:** cache first CEGIS caller context formatting ([#993](https://github.com/vow-lang/vow/issues/993)) ([4699e3c](https://github.com/vow-lang/vow/commit/4699e3c98d870446261a7fcd247c743a96a53f54))

## [0.5.1](https://github.com/vow-lang/vow/compare/v0.5.0...v0.5.1) (2026-07-31)


### Bug Fixes

* address i32 numeric tower review findings from [#976](https://github.com/vow-lang/vow/issues/976) ([#979](https://github.com/vow-lang/vow/issues/979)) ([1c065b6](https://github.com/vow-lang/vow/commit/1c065b6b1da81840af34703935c686d624224d90))

# [0.5.0](https://github.com/vow-lang/vow/compare/v0.4.0...v0.5.0) (2026-07-30)


### Features

* bring i32 to full parity with u8 under the numeric tower ([#525](https://github.com/vow-lang/vow/issues/525)) ([#976](https://github.com/vow-lang/vow/issues/976)) ([f421acd](https://github.com/vow-lang/vow/commit/f421acda67505492848f9a9a3c4c56b6e92d5077))

# [0.4.0](https://github.com/vow-lang/vow/compare/v0.3.0...v0.4.0) (2026-07-23)


### Bug Fixes

* **bootstrap:** warn when no-verify supersedes stage3 flag ([#946](https://github.com/vow-lang/vow/issues/946)) ([516488c](https://github.com/vow-lang/vow/commit/516488c45148904bf83a55f32bd48b3dbcb52863))
* **ci:** drop sed range-block syntax for workspace-member discovery ([#960](https://github.com/vow-lang/vow/issues/960)) ([d2b6a44](https://github.com/vow-lang/vow/commit/d2b6a4486d13360436c4782cb16f3b06d0b23352))
* **ci:** make workspace-version discovery portable to macOS bash 3.2 ([#959](https://github.com/vow-lang/vow/issues/959)) ([78cccd0](https://github.com/vow-lang/vow/commit/78cccd03603b64466b43e61d9ffeba562a2ee667))
* **compiler:** fail closed on unsupported match patterns ([0d422e6](https://github.com/vow-lang/vow/commit/0d422e694d0b4cce601727d6e5d623e4b51442ec))
* **diag:** add unsupported pattern code ([3707aed](https://github.com/vow-lang/vow/commit/3707aedb8ad86c82bdc48365b67f6745b3e54113))
* **diag:** retain diagnostics when inner emission fails ([#941](https://github.com/vow-lang/vow/issues/941)) ([72ef6ff](https://github.com/vow-lang/vow/commit/72ef6ff8925a4a36787ebb6bc7fa5b3a61e589f5))
* **examples/chess:** bound game repetition history ([6250832](https://github.com/vow-lang/vow/commit/6250832bf58a9a84fd62b13e06bfb5a6aa9537a1))
* **examples/chess:** bound repetition history seeding ([90caf84](https://github.com/vow-lang/vow/commit/90caf84be445babde045b0801e68133c0ff3ac33))
* **examples/chess:** complete repetition draw checks ([d8e65f7](https://github.com/vow-lang/vow/commit/d8e65f7d5ff076c529575bbb134706fd2cac41b3))
* **examples/chess:** honor go infinite until stop ([d6ee29c](https://github.com/vow-lang/vow/commit/d6ee29ce0f900bc458c38f49fc34eb4df23df308))
* **examples/chess:** honor go infinite until stop ([#922](https://github.com/vow-lang/vow/issues/922)) ([42e2fef](https://github.com/vow-lang/vow/commit/42e2fefe827ccb6a8ffebbb2cbd41109a00cca6d)), closes [#917](https://github.com/vow-lang/vow/issues/917)
* **examples/chess:** honor quit during go infinite ([eb39c66](https://github.com/vow-lang/vow/commit/eb39c66a9354860823aa147dc0319ce1c09e0af1))
* **examples/chess:** isolate repetition draw context ([a70580b](https://github.com/vow-lang/vow/commit/a70580b1727f40c8f6ba2d36f86464fd162ac1c1))
* **examples/chess:** isolate repetition search contexts ([39583a5](https://github.com/vow-lang/vow/commit/39583a53e5ae52b4a04a80c287eeb67de7544fd4))
* **examples/chess:** keep last exact root move on aspiration fail-low ([698831f](https://github.com/vow-lang/vow/commit/698831f5b9336362fcc61454ee482ee43f46304c))
* **examples/chess:** recognize draws in quiescence ([c88dccf](https://github.com/vow-lang/vow/commit/c88dccf5ea85025465fa4e8cd030145a8ac7941c))
* **examples/chess:** require full threefold history ([c774c7b](https://github.com/vow-lang/vow/commit/c774c7b9c211cc0acb11c7f3bac62f8e71988196))
* **examples/chess:** reserve repetition search headroom ([7dbf0ee](https://github.com/vow-lang/vow/commit/7dbf0ee308372da94ccc682cde7e042b9c3958b4))
* **examples/chess:** restore validator FEN read timeout ([cc56751](https://github.com/vow-lang/vow/commit/cc56751250afd5c2b7d07ac457fc5d13c158c9f2)), closes [#907](https://github.com/vow-lang/vow/issues/907)
* **examples/chess:** score dead positions before quiesce depth cutoff ([329fae1](https://github.com/vow-lang/vow/commit/329fae1561f0d5063555909ed634172a5c2067b8))
* **examples/chess:** seed full repetition history ([#923](https://github.com/vow-lang/vow/issues/923)) ([7f2061c](https://github.com/vow-lang/vow/commit/7f2061cc74819b04868d769e43af3c67142d56fe)), closes [#910](https://github.com/vow-lang/vow/issues/910) [#910](https://github.com/vow-lang/vow/issues/910)
* **examples/chess:** seed search with game repetition history ([557cbda](https://github.com/vow-lang/vow/commit/557cbdacc9abd1c7cb5e65926c64a754016ffd3b))
* **examples/chess:** stop search on stdin EOF ([83a5ebf](https://github.com/vow-lang/vow/commit/83a5ebf9fd1c8530bdbfa3344851b6544c885986))
* **lexer:** specify sibling byte classifiers ([#943](https://github.com/vow-lang/vow/issues/943)) ([8869431](https://github.com/vow-lang/vow/commit/88694310937ed4e90b4e890aef9e3e92bec23309))
* **match:** reject non-final catchall arms ([a9a121f](https://github.com/vow-lang/vow/commit/a9a121f86e3889a47190c2af7c9eb2a419089abc))
* **match:** reject unsupported patterns before lowering ([#920](https://github.com/vow-lang/vow/issues/920)) ([8d1991e](https://github.com/vow-lang/vow/commit/8d1991eb211b9d4212b9fbd68bfc3928bdc973d9)), closes [#903](https://github.com/vow-lang/vow/issues/903)
* **parser:** require match-arm comma separators ([f471614](https://github.com/vow-lang/vow/commit/f471614681ef6bcf40a1841fe31eece6971a9126))
* **parser:** require match-arm comma separators ([#918](https://github.com/vow-lang/vow/issues/918)) ([dfa8cd7](https://github.com/vow-lang/vow/commit/dfa8cd7656594f48261ca29012a819fa5ee0efc7)), closes [#904](https://github.com/vow-lang/vow/issues/904)
* **types:** reject unsafe match patterns before lowering ([720d437](https://github.com/vow-lang/vow/commit/720d437849929f4da367b52e8c57e4f47cfd5cfd))
* **types:** treat match bindings as catchalls ([1ce207c](https://github.com/vow-lang/vow/commit/1ce207c484d80de2d10655fecf2fa1c227a29622))
* **verify:** eliminate fake ESBMC write-exec race ([b0ae065](https://github.com/vow-lang/vow/commit/b0ae0652aa19625de68b434a9db8c00ed57cb49f))
* **verify:** eliminate fake ESBMC write-exec race ([#919](https://github.com/vow-lang/vow/issues/919)) ([ad861e2](https://github.com/vow-lang/vow/commit/ad861e2d2f79641aa04f817897c9a17ee32bd2e7)), closes [#915](https://github.com/vow-lang/vow/issues/915)
* **vow-verify:** skip complexity descriptor IR nodes ([#945](https://github.com/vow-lang/vow/issues/945)) ([a0b388a](https://github.com/vow-lang/vow/commit/a0b388ae59dd508f12789dd5b7b46765a19bb29f))
* **vow:** handle frontend diagnostic I/O failures ([#944](https://github.com/vow-lang/vow/issues/944)) ([7b302b3](https://github.com/vow-lang/vow/commit/7b302b39aa03f2cafb3124f0358b5e4dfb95eaaa))


### Features

* **examples/chess:** add lightweight endgame knowledge ([#924](https://github.com/vow-lang/vow/issues/924)) ([eb0a2f8](https://github.com/vow-lang/vow/commit/eb0a2f8f3072f7ff464bbda1c5d991b7d579f85a)), closes [#909](https://github.com/vow-lang/vow/issues/909)
* **examples/chess:** complete basic-mate mop-up knowledge ([9d8eff4](https://github.com/vow-lang/vow/commit/9d8eff492412ba784e24dc5f4ff7805f91d654aa))
* **examples/chess:** deepen search with selective pruning ([a025f90](https://github.com/vow-lang/vow/commit/a025f902e81761fc9dbbf673eafb8f105b31e993))
* **examples/chess:** deepen search with selective pruning ([#930](https://github.com/vow-lang/vow/issues/930)) ([37d0aa8](https://github.com/vow-lang/vow/commit/37d0aa8a17f4ff005636e6e9b1786284643ff1e8)), closes [#911](https://github.com/vow-lang/vow/issues/911)
* **examples/chess:** detect insufficient-material draws ([264310e](https://github.com/vow-lang/vow/commit/264310ee4fc1bfb84d8c379c4ca1995669997d34))
* **examples/chess:** guide KQ mating conversion ([8c8d836](https://github.com/vow-lang/vow/commit/8c8d836b319e3e78c3b294bd8346efe68c54b54f))
* **examples/chess:** strengthen UCI engine from ~1520 to ~2110 Elo ([352a52f](https://github.com/vow-lang/vow/commit/352a52ff47eb7f569563f25785b13af5ae6a7b06))
* **examples/chess:** strengthen UCI engine to ~2110 Elo ([#907](https://github.com/vow-lang/vow/issues/907)) ([6b379c9](https://github.com/vow-lang/vow/commit/6b379c9a01f940aa21f4e9568345d57b83aad5f1)), closes [#879](https://github.com/vow-lang/vow/issues/879) [#908](https://github.com/vow-lang/vow/issues/908) [#909](https://github.com/vow-lang/vow/issues/909) [#910](https://github.com/vow-lang/vow/issues/910) [#911](https://github.com/vow-lang/vow/issues/911) [#912](https://github.com/vow-lang/vow/issues/912)
* **numeric:** make u8 first-class end to end ([#937](https://github.com/vow-lang/vow/issues/937)) ([e64253e](https://github.com/vow-lang/vow/commit/e64253e9a5ef6b9add094c23f74a77d17e1986f7))


### Performance Improvements

* **examples/chess:** gate endgame scans by material ([d47ad4d](https://github.com/vow-lang/vow/commit/d47ad4d09a605ff8734a6724dd93a9f3c0263258))
* **examples/chess:** gate mop_up_score on non_king_count too ([36530d0](https://github.com/vow-lang/vow/commit/36530d062b8015ebda759fe757e089c9792f0453))
* **examples/chess:** reuse computed move_key in negamax best update ([51f8826](https://github.com/vow-lang/vow/commit/51f88267956b27bbaf231a05c87812d30b71c7b7))
