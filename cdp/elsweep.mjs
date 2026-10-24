import { execFileSync } from "node:child_process";
const R = 130;
for (const el of [-25, 0, 45, 65]) {
  for (let i = 0; i < 8; i++) {
    const az = (i * Math.PI) / 4 + Math.PI / 8;
    const pos = `${(R*Math.cos(el*Math.PI/180)*Math.cos(az)).toFixed(1)},${(R*Math.sin(el*Math.PI/180)).toFixed(1)},${(R*Math.cos(el*Math.PI/180)*Math.sin(az)).toFixed(1)}`;
    const expr = `(()=>{const c=window.__viewer.camera;c.position.set(${pos});c.lookAt(0,-5,0);c.updateProjectionMatrix();return 1;})()`;
    execFileSync("node", ["drive.mjs", expr], { encoding: "utf8" });
    execFileSync("node", ["drive.mjs", "--shot", `../target/fm38_el.png`], { encoding: "utf8" });
    const cnt = execFileSync("python", ["-c",
      `from PIL import Image;im=Image.open('../target/fm38_el.png').convert('RGB');w,h=im.size;n=sum(1 for y in range(0,h,3) for x in range(0,w,3) if (lambda r,g,b: r>75 and b>60 and g<r*0.68 and g<b*0.78 and r+g+b>170)(*im.getpixel((x,y))));print(n)`],
      { encoding: "utf8" }).trim();
    if (+cnt > 20) console.log(`el=${el} az=${i} purple=${cnt}`);
  }
}
console.log('sweep done');
