# Wiki pages

These files are the content of the [Kura wiki](https://github.com/Koxility/kura/wiki). They live here so changes are reviewed together with the code. To publish them:

```bash
git clone https://github.com/Koxility/kura.wiki.git /tmp/kura-wiki
cp docs/wiki/*.md /tmp/kura-wiki/ && rm /tmp/kura-wiki/README.md
git -C /tmp/kura-wiki add -A && git -C /tmp/kura-wiki commit -m "Update the documentation"
git -C /tmp/kura-wiki push
```
