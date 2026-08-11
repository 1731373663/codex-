@echo off
chcp 65001 >nul
set "CARGO=%USERPROFILE%\.cargo\bin\cargo.exe"
if not exist "%CARGO%" (
    echo CARGO_NOT_FOUND > check_output.txt
    exit /b 1
)
echo === cargo check codex-plus-core === > check_output.txt
"%CARGO%" check -p codex-plus-core 2>> check_output.txt
echo CORE_EXIT=%ERRORLEVEL% >> check_output.txt

echo === cargo check codex-plus-manager === >> check_output.txt
"%CARGO%" check -p codex-plus-manager 2>> check_output.txt
echo MANAGER_EXIT=%ERRORLEVEL% >> check_output.txt
