const fs = require("fs");
const path = require("path");

// CHANGE THIS TO YOUR DIRECTORY PATH
const folder = "/sdcard/Demon Slayer 2";
const outputFilename = "Demon.Slayer.Mugen.Train.FULL.mkv";

const allFiles = fs.readdirSync(folder);

// Filter part files
const partFiles = allFiles
  .filter(file => file.includes(".mkv.part"))
  .sort((a, b) => {
    const aNum = parseInt(a.match(/part(\d+)$/)[1], 10);
    const bNum = parseInt(b.match(/part(\d+)$/)[1], 10);
    return aNum - bNum;
  });

const outputPath = path.join(folder, outputFilename);
const writeStream = fs.createWriteStream(outputPath);

function mergeParts(files, index = 0) {
  if (index >= files.length) {
    writeStream.end();
    console.log(`✅ Done! Merged ${files.length} parts into "${outputFilename}"`);
    return;
  }

  const currentPath = path.join(folder, files[index]);
  const readStream = fs.createReadStream(currentPath);

  console.log(`📦 Merging: ${files[index]}`);
  readStream.pipe(writeStream, { end: false });

  readStream.on("end", () => mergeParts(files, index + 1));
  readStream.on("error", err => {
    console.error(`❌ Error reading ${files[index]}:`, err);
    writeStream.destroy();
  });
}

mergeParts(partFiles);
