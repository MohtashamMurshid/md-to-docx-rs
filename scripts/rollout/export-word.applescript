on run argv
  set inputPath to item 1 of argv
  set outputPath to item 2 of argv
  tell application "Microsoft Word"
    set testDocument to open file name inputPath read only true add to recent files false
    try
      save as testDocument file name outputPath file format format PDF
      close testDocument saving no
    on error errorMessage number errorNumber
      try
        close testDocument saving no
      end try
      error errorMessage number errorNumber
    end try
  end tell
end run
