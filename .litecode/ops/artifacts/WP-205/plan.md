目标：修复 session_search 3 个既有失败测试，确认溢出文件可读、hit 行与虚拟 read 一致。
根因（见 dig-survey.md）：测试过时。bd71f8c 删了 footer 的 " — read or grep it"，测试仍按 " —" 截路径；c688b0a(WP-199) read 改为结构化 data 并退役 end_line，测试仍读 content/传 end_line。
方案：Tools 域，仅改测试：共用 helper 解析 "are in <path>."（去尾句号）；虚拟 read 测试改 start_line+limit、断言 data.lines。可选 footer 路径加反引号。不改 read/spill 产品逻辑。
验收：结果点名的溢出文件真能读到；hit 行与 read 一致；3 测变绿且 session_search 过滤全绿（cargo -j2，内存≥2GiB）；dig 写清根因；推实验分支 feat/tool-contract-ptc，不合 main。
