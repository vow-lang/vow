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
