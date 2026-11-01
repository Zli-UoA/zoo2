package zoo.android.update;
import android.os.ParcelFileDescriptor;
interface IUpdateInstaller {
    void destroy() = 16777114;
    String install(in ParcelFileDescriptor apk, long size, int userId) = 1;
}
