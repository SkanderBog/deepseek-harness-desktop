## crates 工作流程

```
graph TD
    A[cd crates/repo-a<br/>修改代码并 push/PR 远端] --> B[cd ..<br/>返回根目录]
    B --> C[git add crates/repo-a<br/>暂存子模块更新指针]
    C --> D[git commit && git push<br/>提交并推送主仓]
```

git submodule add https://github.com/hairyf/tauri-plugin-notifications crates/