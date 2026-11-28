package zoo.android.ui

import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@Composable
internal fun ProfileHeader(profile: AndroidScreen.Profile, colors: AndroidColors, follow: () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(bottom = 4.dp)) {
        Box(Modifier.fillMaxWidth().height(92.dp)) {
            Box(Modifier.fillMaxWidth().height(72.dp).clip(RoundedCornerShape(20.dp))
                .background(Brush.horizontalGradient(listOf(colors.headerLight, colors.quietSurface))))
            Box(
                Modifier.align(Alignment.BottomStart).padding(start = 12.dp).size(72.dp)
                    .background(colors.background, CircleShape).padding(4.dp)
                    .clip(CircleShape).background(colors.headerLight),
                contentAlignment = Alignment.Center,
            ) {
                val avatar = profile.avatar
                if (avatar != null) Image(avatar, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
                else Text(profile.name.take(1), color = colors.foreground, fontSize = 26.sp,
                    fontWeight = AndroidWeights.emphasis)
            }
            profile.follow?.let { state ->
                TextAction(androidLabel(state.key, state.fallback), colors, modifier = Modifier.align(Alignment.BottomEnd).padding(end = 8.dp),
                    icon = if (state == FollowState.Follow) UiIcon.UserAdd else UiIcon.Close,
                    style = if (state == FollowState.Follow) TextActionStyle.Primary else TextActionStyle.Neutral, click = follow)
            }
        }
        Column(Modifier.fillMaxWidth().padding(horizontal = 4.dp).padding(top = 5.dp)) {
            Text(profile.name, color = colors.foreground, fontSize = 25.sp, lineHeight = 30.sp,
                fontWeight = AndroidWeights.emphasis, maxLines = 2, overflow = TextOverflow.Ellipsis)
            profile.handle?.let { handle ->
                Text(handle, color = colors.secondary, fontSize = 14.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
            }
            if (profile.bio.isNotBlank()) {
                Text(formatEntry(profile.bio), Modifier.padding(top = 10.dp), color = colors.foreground,
                    fontSize = 15.sp, lineHeight = 22.sp)
            }
            Text(androidLabel("android.profile.posts", "投稿"), Modifier.padding(top = 18.dp, bottom = 4.dp), color = colors.foreground,
                fontWeight = AndroidWeights.emphasis, fontSize = 16.sp)
            if (profile.posts.isEmpty() && !profile.postsPending) {
                Text(androidLabel("android.profile.noPosts", "表示できる投稿はありません"), Modifier.padding(vertical = 16.dp), color = colors.secondary, fontSize = 14.sp)
            }
        }
    }
}
