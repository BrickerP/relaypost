# v0.1 评审收尾

本轮收敛五项 P2 问题，复用现有 Rust 核心、目录结构和 SQLite JSON 记录。未新增 SQL 表，未加入旧 acknowledgment 格式的回退解析。

| 原问题 | 最终行为 | 回归覆盖 |
| --- | --- | --- |
| FIFO 等特殊文件阻塞载体检查 | 打开前拒绝非普通文件；Unix 非阻塞打开后再次核对描述符类型 | 文件系统单元用例，以及有超时保护的包裹／回执 FIFO 集成用例 |
| 中断暂存无法继续或清理 | 完整 prepared 批次复核后继续发布；与有效最终批次对应的残留副本在同步后清理；部分或冲突批次保留待用户显式丢弃 | 发布前恢复、发布后仅剩归档／密文的真实目录形态、冲突保留、丢弃范围 |
| 本地分片损坏阻止恢复签收 | acknowledgment 固定保存回执和原始签名信封；恢复时独立验证，先重建 Confirmed，再报告本地副本健康 | 索引陈旧／丢失与 outbox 损坏／丢失组合，以及四个证据文件的篡改拒绝 |
| 修复副本后丢失待回执阶段 | 发件 `state` 保留投递事实，`local_error` 单独记录健康问题；恢复只清除健康告警 | Ready、WaitingReceipt、Confirmed、Cancelled 的损坏与恢复 |
| 发给自身导致记录选错 | 核心拒绝自身联系人、自发自收；界面记录以 `(parcel_id, direction)` 定位，CLI 取消结果限定发件方向 | 自身导入、信任、发送拒绝；CLI 完整收发演示 |

## 使用入口

History 的恢复区域提供 **Verify & recover local records** 与 **Discard interrupted receive work…**。丢弃需要在界面确认，后台只处理当前资料库拥有的接收暂存目录。CLI 使用 `recover --discard-incomplete`，只执行丢弃并返回 `discarded_staging` 数量，不需要密码。

发件历史中，签收或待回执阶段与本地副本告警同时显示。原文件、载体邮包和已提交批次均不属于丢弃目标。

## 验证与边界

Rust 编译、格式检查、Clippy、单元与集成回归，以及真实 CLI 演示均由 [GitHub Actions](https://github.com/BrickerP/relaypost/actions/workflows/ci.yml) 执行，当前提交的通过状态见 [CR #1](https://github.com/BrickerP/relaypost/pull/1/checks)。本轮未本地编译或运行 Rust 测试。

回归通过构造合成身份和持久目录状态覆盖中断窗口。真实双机、物理 USB、断电行为、独立安全审计、Apple 公证，以及 Kiro 的真实开发记录仍需各自完成；现有记录不代替这些验证。
